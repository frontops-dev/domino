use crate::error::Result;
use crate::git;
use crate::profiler::Profiler;
use crate::semantic::{ReferenceFinder, WorkspaceAnalyzer};
use crate::types::{
  AffectCause, AffectedProjectInfo, AffectedReport, AffectedResult, Project, TrueAffectedConfig,
};
use crate::utils;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::debug;

/// Mutable state for tracking affected symbols during analysis
struct AffectedState<'a> {
  affected_packages: &'a mut FxHashSet<String>,
  project_causes: Option<&'a mut FxHashMap<String, Vec<AffectCause>>>,
  visited: &'a mut FxHashSet<(PathBuf, String)>,
}

/// Main true-affected algorithm implementation
pub fn find_affected(
  config: TrueAffectedConfig,
  profiler: Arc<Profiler>,
) -> Result<AffectedResult> {
  find_affected_internal(config, profiler, false)
}

/// Main true-affected algorithm implementation with optional report generation
pub fn find_affected_with_report(
  config: TrueAffectedConfig,
  profiler: Arc<Profiler>,
) -> Result<AffectedResult> {
  find_affected_internal(config, profiler, true)
}

fn find_affected_internal(
  config: TrueAffectedConfig,
  profiler: Arc<Profiler>,
  generate_report: bool,
) -> Result<AffectedResult> {
  debug!("Starting true-affected analysis");
  debug!("Base: {}", config.base);
  debug!("Projects: {}", config.projects.len());

  // Step 1: Get changed files from git
  let changed_files = git::get_changed_files(&config.cwd, &config.base)?;
  debug!("Found {} changed files", changed_files.len());

  if changed_files.is_empty() {
    debug!("No changes detected");
    return Ok(AffectedResult {
      affected_projects: vec![],
      report: None,
    });
  }

  // Step 2: Build workspace analyzer (includes building import index)
  debug!("Building workspace semantic analysis...");
  let analyzer = WorkspaceAnalyzer::new(config.projects.clone(), &config.cwd, profiler.clone())?;
  debug!("Analyzed {} files", analyzer.files.len());

  // Step 3: Initialize reference finder
  let reference_finder = ReferenceFinder::new(&analyzer, &config.cwd, profiler.clone());

  // Step 4: Track affected packages and their causes
  let mut affected_packages = FxHashSet::default();
  let mut project_causes: FxHashMap<String, Vec<AffectCause>> = FxHashMap::default();

  // Step 5: Process each changed file and line
  for changed_file in &changed_files {
    let file_path = &changed_file.file_path;

    // Check if file exists in our analyzed files
    if !analyzer.files.contains_key(file_path) {
      debug!("Skipping non-source file: {:?}", file_path);
      continue;
    }

    // Add the package that owns this file
    if let Some(pkg) = utils::get_package_name_by_path(file_path, &config.projects) {
      debug!("File {:?} belongs to package '{}'", file_path, pkg);
      affected_packages.insert(pkg.clone());

      // Record direct change cause if generating report
      if generate_report {
        // For each changed line, record it as a direct change
        for &line in &changed_file.changed_lines {
          let symbol = analyzer
            .find_node_at_line(file_path, line, 0)
            .ok()
            .flatten();
          project_causes
            .entry(pkg.clone())
            .or_default()
            .push(AffectCause::DirectChange {
              file: file_path.clone(),
              symbol,
              line,
            });
        }
      }
    }

    // Process each changed line
    for &line in &changed_file.changed_lines {
      if let Err(e) = process_changed_line(
        &analyzer,
        &reference_finder,
        file_path,
        line,
        &config.projects,
        &mut affected_packages,
        if generate_report {
          Some(&mut project_causes)
        } else {
          None
        },
      ) {
        debug!("Error processing line {} in {:?}: {}", line, file_path, e);
        // Continue processing other lines
      }
    }
  }

  // Step 6: Add implicit dependencies
  add_implicit_dependencies(
    &config.projects,
    &mut affected_packages,
    if generate_report {
      Some(&mut project_causes)
    } else {
      None
    },
  );

  // Step 7: Convert to sorted vector
  let mut affected_projects: Vec<String> = affected_packages.into_iter().collect();
  affected_projects.sort();

  debug!("Affected projects: {:?}", affected_projects);

  // Step 8: Build report if requested
  let report = if generate_report {
    let mut projects_info: Vec<AffectedProjectInfo> = project_causes
      .into_iter()
      .map(|(name, mut causes)| {
        // Deduplicate causes - sort and remove duplicates
        causes.sort();
        causes.dedup();
        AffectedProjectInfo { name, causes }
      })
      .collect();
    projects_info.sort_by(|a, b| a.name.cmp(&b.name));

    Some(AffectedReport {
      projects: projects_info,
    })
  } else {
    None
  };

  // Print profiling report if enabled
  profiler.print_report();

  Ok(AffectedResult {
    affected_projects,
    report,
  })
}

fn process_changed_line(
  analyzer: &WorkspaceAnalyzer,
  reference_finder: &ReferenceFinder,
  file_path: &Path,
  line: usize,
  projects: &[Project],
  affected_packages: &mut FxHashSet<String>,
  project_causes: Option<&mut FxHashMap<String, Vec<AffectCause>>>,
) -> Result<()> {
  // Find the node at this line
  let symbol_name = match analyzer.find_node_at_line(file_path, line, 0)? {
    Some(name) => name,
    None => {
      debug!("No symbol found at line {} in {:?}", line, file_path);
      return Ok(());
    }
  };

  debug!("Processing symbol '{}' in {:?}", symbol_name, file_path);

  // Use a visited set to avoid infinite recursion
  let mut visited = FxHashSet::default();
  let mut state = AffectedState {
    affected_packages,
    project_causes,
    visited: &mut visited,
  };
  process_changed_symbol(
    analyzer,
    reference_finder,
    file_path,
    &symbol_name,
    projects,
    &mut state,
  )?;

  Ok(())
}

fn process_changed_symbol(
  analyzer: &WorkspaceAnalyzer,
  reference_finder: &ReferenceFinder,
  file_path: &Path,
  symbol_name: &str,
  projects: &[Project],
  state: &mut AffectedState,
) -> Result<()> {
  // Avoid infinite recursion
  let key = (file_path.to_path_buf(), symbol_name.to_string());
  if state.visited.contains(&key) {
    return Ok(());
  }
  state.visited.insert(key);

  debug!("Processing symbol '{}' in {:?}", symbol_name, file_path);

  // Get the source project for causality tracking
  let source_project = utils::get_package_name_by_path(file_path, projects);

  // 1. Find local references in the same file
  let local_refs = analyzer.find_local_references(file_path, symbol_name)?;
  debug!(
    "Found {} local references for '{}'",
    local_refs.len(),
    symbol_name
  );

  for local_ref in local_refs {
    // Find the root symbol containing this reference
    if let Some(container_symbol) =
      analyzer.find_node_at_line(file_path, local_ref.line, local_ref.column)?
    {
      // Skip if it's the same symbol (self-reference)
      if container_symbol != symbol_name {
        debug!(
          "Local reference in '{}' at line {}",
          container_symbol, local_ref.line
        );
        // Recursively process the containing symbol
        process_changed_symbol(
          analyzer,
          reference_finder,
          file_path,
          &container_symbol,
          projects,
          state,
        )?;
      }
    }
  }

  // 2. Find cross-file references (includes exported symbols)
  let cross_file_refs = reference_finder.find_cross_file_references(symbol_name, file_path)?;
  debug!(
    "Found {} cross-file references for '{}'",
    cross_file_refs.len(),
    symbol_name
  );

  // 3. Handle internal (non-exported) symbols with no cross-file references
  // This is critical for tracking transitive dependencies through exported containers.
  //
  // Example scenario:
  //   - Internal function `helperFn()` is modified (no cross-file refs, not exported)
  //   - Exported component `MyComponent` uses `helperFn()`
  //   - Other files import and use `MyComponent`
  //
  // Without this check, we'd miss that `MyComponent` is affected, and thus miss
  // all projects that depend on `MyComponent`. This matches TypeScript's behavior
  // where findAllReferences tracks symbols through their exported containers.
  //
  // We skip exported symbols here because they're already handled by cross-file
  // reference tracking in step 2 above.
  if cross_file_refs.is_empty() && !analyzer.is_symbol_exported(file_path, symbol_name) {
    debug!(
      "Symbol '{}' has no cross-file references and is not exported. Checking if exported symbols use it.",
      symbol_name
    );

    let exported_symbols_using = analyzer.find_exported_symbols_using(file_path, symbol_name)?;
    debug!(
      "Found {} exported symbols using '{}': {:?}",
      exported_symbols_using.len(),
      symbol_name,
      exported_symbols_using
    );

    // Recursively process each exported symbol that uses this local symbol
    // This propagates the change through the export boundary
    for exported_symbol in exported_symbols_using {
      process_changed_symbol(
        analyzer,
        reference_finder,
        file_path,
        &exported_symbol,
        projects,
        state,
      )?;
    }
  }

  // For each cross-file reference, recursively process the containing symbol in that file
  for reference in cross_file_refs {
    // Mark the package as affected
    if let Some(pkg) = utils::get_package_name_by_path(&reference.file_path, projects) {
      state.affected_packages.insert(pkg.clone());

      // Track cause if generating report
      if let Some(ref mut causes_map) = state.project_causes {
        if let Some(ref src_proj) = source_project {
          causes_map
            .entry(pkg.clone())
            .or_default()
            .push(AffectCause::ImportedSymbol {
              source_project: src_proj.clone(),
              symbol: symbol_name.to_string(),
              via_file: reference.file_path.clone(),
              source_file: file_path.to_path_buf(),
            });
        }
      }
    }

    // Special case: line=0,column=0 is a sentinel for "entire file affected" (from dynamic imports)
    // In this case, we need to process all exports from that file
    if reference.line == 0 && reference.column == 0 {
      debug!(
        "File {:?} is conservatively affected (dynamic import). Processing all its exports.",
        reference.file_path
      );

      // Get all exports from the affected file
      if let Some(exports) = analyzer.exports.get(&reference.file_path) {
        for export in exports {
          // Skip re-exports - those are handled separately
          if export.re_export_from.is_some() {
            continue;
          }

          // Get the local name (what's actually defined in the file)
          let local_name = export.local_name.as_ref().unwrap_or(&export.exported_name);

          debug!(
            "Processing exported symbol '{}' from conservatively affected file {:?}",
            local_name, reference.file_path
          );

          // Recursively process this exported symbol
          process_changed_symbol(
            analyzer,
            reference_finder,
            &reference.file_path,
            local_name,
            projects,
            state,
          )?;
        }
      }
    } else {
      // Normal case: find the root symbol containing this reference in the other file
      if let Ok(Some(container_symbol)) =
        analyzer.find_node_at_line(&reference.file_path, reference.line, reference.column)
      {
        debug!(
          "Cross-file reference in '{}' at {:?}:{}",
          container_symbol, reference.file_path, reference.line
        );
        // Recursively process the containing symbol in the importing file
        process_changed_symbol(
          analyzer,
          reference_finder,
          &reference.file_path,
          &container_symbol,
          projects,
          state,
        )?;
      }
    }
  }

  Ok(())
}

fn add_implicit_dependencies(
  projects: &[Project],
  affected_packages: &mut FxHashSet<String>,
  mut project_causes: Option<&mut FxHashMap<String, Vec<AffectCause>>>,
) {
  // Build a map of package -> implicit dependents
  let mut implicit_dep_map: HashMap<String, Vec<String>> = HashMap::new();

  for project in projects {
    if !project.implicit_dependencies.is_empty() {
      for dep in &project.implicit_dependencies {
        implicit_dep_map
          .entry(dep.clone())
          .or_default()
          .push(project.name.clone());
      }
    }
  }

  // For each affected package, add its implicit dependents
  let affected_clone: Vec<String> = affected_packages.iter().cloned().collect();

  for pkg in affected_clone {
    if let Some(dependents) = implicit_dep_map.get(&pkg) {
      debug!("Adding implicit dependents for '{}': {:?}", pkg, dependents);
      for dependent in dependents {
        affected_packages.insert(dependent.clone());

        // Track implicit dependency cause if generating report
        if let Some(ref mut causes_map) = project_causes {
          causes_map
            .entry(dependent.clone())
            .or_default()
            .push(AffectCause::ImplicitDependency {
              depends_on: pkg.clone(),
            });
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  #[test]
  fn test_add_implicit_dependencies() {
    let projects = vec![
      Project {
        name: "app".to_string(),
        source_root: PathBuf::from("apps/app"),
        ts_config: None,
        implicit_dependencies: vec!["lib1".to_string(), "lib2".to_string()],
        targets: vec![],
      },
      Project {
        name: "lib1".to_string(),
        source_root: PathBuf::from("libs/lib1"),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "lib2".to_string(),
        source_root: PathBuf::from("libs/lib2"),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
    ];

    let mut affected = FxHashSet::default();
    affected.insert("lib1".to_string());

    add_implicit_dependencies(&projects, &mut affected, None);

    assert!(affected.contains("lib1"));
    assert!(affected.contains("app")); // Should be added as implicit dependent
  }
}
