use crate::error::Result;
use crate::profiler::Profiler;
use crate::semantic::WorkspaceAnalyzer;
use crate::types::Reference;
use oxc_resolver::{ResolveOptions, Resolver};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, warn};

/// Cross-file reference finder
pub struct ReferenceFinder<'a> {
  analyzer: &'a WorkspaceAnalyzer,
  resolver: Resolver,
  cwd: PathBuf,
  /// Resolution cache: (from_file, specifier) -> resolved_path
  /// Using RefCell for interior mutability since resolution is logically const
  /// Note: Not thread-safe. For future parallelization, migrate to DashMap or Arc<Mutex<>>
  resolution_cache: RefCell<FxHashMap<(PathBuf, String), Option<PathBuf>>>,
  /// Profiler for performance measurement
  profiler: Arc<Profiler>,
}

impl<'a> ReferenceFinder<'a> {
  pub fn new(analyzer: &'a WorkspaceAnalyzer, cwd: &Path, profiler: Arc<Profiler>) -> Self {
    // Try to find tsconfig.base.json for path mappings
    let tsconfig_path = cwd.join("tsconfig.base.json");

    let options = ResolveOptions {
      extensions: vec![
        ".ts".into(),
        ".tsx".into(),
        ".js".into(),
        ".jsx".into(),
        ".d.ts".into(),
      ],
      tsconfig: if tsconfig_path.exists() {
        Some(oxc_resolver::TsconfigDiscovery::Manual(
          oxc_resolver::TsconfigOptions {
            config_file: tsconfig_path.clone(),
            references: oxc_resolver::TsconfigReferences::Auto,
          },
        ))
      } else {
        None
      },
      ..Default::default()
    };

    Self {
      analyzer,
      resolver: Resolver::new(options),
      cwd: cwd.to_path_buf(),
      resolution_cache: RefCell::new(FxHashMap::default()),
      profiler,
    }
  }

  /// Find all files that import from the given file (regardless of what symbol)
  #[allow(dead_code)]
  pub fn find_files_importing_from(&self, file_path: &Path) -> Result<Vec<Reference>> {
    let mut importing_files = Vec::new();

    debug!("Finding all files importing from {:?}", file_path);

    for (importing_file, file_imports) in &self.analyzer.imports {
      for import in file_imports {
        // Resolve the import to see if it points to file_path
        let resolved = self.resolve_import(importing_file, &import.from_module);

        if let Some(resolved_path) = resolved {
          if self.paths_equal(&resolved_path, file_path) {
            debug!("Found import in {:?}", importing_file);
            importing_files.push(Reference {
              file_path: importing_file.clone(),
              line: 0,
              column: 0,
            });
            break; // Only add each file once
          }
        }
      }
    }

    Ok(importing_files)
  }

  /// Find all cross-file references to a symbol
  pub fn find_cross_file_references(
    &self,
    symbol_name: &str,
    declaring_file: &Path,
  ) -> Result<Vec<Reference>> {
    let mut all_refs = Vec::new();
    let mut visited = FxHashSet::default();

    self.find_refs_recursive(symbol_name, declaring_file, &mut all_refs, &mut visited)?;

    Ok(all_refs)
  }

  fn find_refs_recursive(
    &self,
    symbol_name: &str,
    current_file: &Path,
    all_refs: &mut Vec<Reference>,
    visited: &mut FxHashSet<(PathBuf, String)>,
  ) -> Result<()> {
    let key = (current_file.to_path_buf(), symbol_name.to_string());
    if !visited.insert(key.clone()) {
      return Ok(()); // Already processed
    }

    debug!(
      "Finding references to '{}' from {:?}",
      symbol_name, current_file
    );

    // Record reference lookup
    self.profiler.record_reference_lookup();

    // Use the import index to find direct imports of this symbol
    if let Some(importers) = self.analyzer.import_index.get(&key) {
      for (importing_file, local_name, _from_module) in importers {
        debug!(
          "Found import of '{}' in {:?} as '{}'",
          symbol_name, importing_file, local_name
        );

        // Find all references to the local name in the importing file
        match self
          .analyzer
          .find_local_references(importing_file, local_name)
        {
          Ok(local_refs) => {
            all_refs.extend(local_refs);
          }
          Err(e) => {
            warn!("Error finding local references: {}", e);
          }
        }

        // Check if it's re-exported
        if self.is_re_exported(importing_file, local_name) {
          debug!(
            "Symbol '{}' is re-exported from {:?}",
            local_name, importing_file
          );
          // Recursively find references to the re-export
          self.find_refs_recursive(local_name, importing_file, all_refs, visited)?;
        } else {
          // Symbol is used but not re-exported
          // The references found via find_local_references above are sufficient
          // The cascade will happen naturally in core.rs when processing
          // the container symbols that actually use this symbol
          debug!(
            "Symbol '{}' is used in {:?} (not re-exported)",
            local_name, importing_file
          );
        }
      }
    }

    // Also check for namespace imports (import * as foo)
    let namespace_key = (current_file.to_path_buf(), "*".to_string());
    if let Some(importers) = self.analyzer.import_index.get(&namespace_key) {
      for (importing_file, local_name, _from_module) in importers {
        debug!(
          "Found namespace import in {:?} as '{}' (checking for {}.{})",
          importing_file, local_name, local_name, symbol_name
        );

        // For namespace imports, we need to find references to namespace.symbol
        // This is more complex - for now, we'll mark the whole file as potentially affected
        // TODO: Improve this by finding actual property accesses
        match self
          .analyzer
          .find_local_references(importing_file, local_name)
        {
          Ok(local_refs) => {
            all_refs.extend(local_refs);
          }
          Err(e) => {
            warn!("Error finding local references: {}", e);
          }
        }
      }
    }

    // Check for re-exports from the same package (barrel files)
    // We need to check files in the same package that might re-export this symbol
    if let Some(exports) = self.analyzer.exports.get(current_file) {
      for export in exports {
        // Skip if not our symbol
        if export.exported_name != symbol_name && export.local_name.as_deref() != Some(symbol_name)
        {
          continue;
        }

        // If this is a re-export from elsewhere, follow it
        if let Some(ref from_module) = export.re_export_from {
          if let Some(resolved) = self.resolve_import(current_file, from_module) {
            debug!(
              "Following re-export of '{}' from {:?} to {:?}",
              symbol_name, current_file, resolved
            );
            self.find_refs_recursive(symbol_name, &resolved, all_refs, visited)?;
          }
        }
      }
    }

    // REVERSE: Find files that re-export FROM the current file (barrel files like index.ts)
    // For example, if clients.module.ts exports ClientsModule, and index.ts re-exports it,
    // we need to look for imports of index.ts
    for (reexporting_file, file_exports) in &self.analyzer.exports {
      for export in file_exports {
        // Check if this export is a re-export from our current_file
        if let Some(ref from_module) = export.re_export_from {
          if let Some(resolved) = self.resolve_import(reexporting_file, from_module) {
            if self.paths_equal(&resolved, current_file) {
              // Handle wildcard re-exports: export * from '...'
              if export.exported_name == "*" {
                debug!(
                  "Found barrel file {:?} with wildcard re-export from {:?}",
                  reexporting_file, current_file
                );
                // Recursively look for imports of the re-exporting file
                // The symbol name stays the same through wildcard re-exports
                self.find_refs_recursive(symbol_name, reexporting_file, all_refs, visited)?;
              } else {
                // Named re-export: export { X } from '...' or export { X as Y } from '...'
                let exported_symbol = export
                  .local_name
                  .as_deref()
                  .unwrap_or(&export.exported_name);
                if exported_symbol == symbol_name {
                  debug!(
                    "Found barrel file {:?} re-exporting '{}' from {:?}",
                    reexporting_file, export.exported_name, current_file
                  );
                  // Recursively look for imports of the re-exporting file
                  self.find_refs_recursive(
                    &export.exported_name,
                    reexporting_file,
                    all_refs,
                    visited,
                  )?;
                }
              }
            }
          }
        }
      }
    }

    Ok(())
  }

  /// Resolve an import specifier to a file path (with caching)
  fn resolve_import(&self, from_file: &Path, specifier: &str) -> Option<PathBuf> {
    let start = if self.profiler.is_enabled() {
      Some(Instant::now())
    } else {
      None
    };

    let cache_key = (from_file.to_path_buf(), specifier.to_string());

    // Check cache first
    let cache_hit = {
      let cache = self.resolution_cache.borrow();
      if let Some(cached) = cache.get(&cache_key) {
        if let Some(start_time) = start {
          self
            .profiler
            .record_resolution(true, start_time.elapsed().as_nanos() as u64);
        }
        return cached.clone();
      }
      false
    };

    // Not in cache, resolve it
    let from_path = self.cwd.join(from_file);
    let context = from_path.parent()?;

    let resolved = match self.resolver.resolve(context, specifier) {
      Ok(resolution) => {
        let resolved = resolution.path();
        // Make it relative to cwd
        resolved
          .strip_prefix(&self.cwd)
          .ok()
          .map(|p| p.to_path_buf())
      }
      Err(_) => {
        // Try simple relative resolution as fallback
        self.simple_resolve(context, specifier)
      }
    };

    // Cache the result (even if None)
    self
      .resolution_cache
      .borrow_mut()
      .insert(cache_key, resolved.clone());

    if let Some(start_time) = start {
      self
        .profiler
        .record_resolution(cache_hit, start_time.elapsed().as_nanos() as u64);
    }

    resolved
  }

  /// Simple fallback resolution for relative imports
  fn simple_resolve(&self, context: &Path, specifier: &str) -> Option<PathBuf> {
    if !specifier.starts_with('.') {
      return None;
    }

    let base = context.join(specifier);

    // Try with different extensions
    for ext in &[".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.js"] {
      let candidate = if ext.starts_with('/') {
        base.join(ext.trim_start_matches('/'))
      } else {
        // Append extension instead of replacing it
        // This handles cases like colors.css -> colors.css.ts (vanilla-extract)
        PathBuf::from(format!("{}{}", base.display(), ext))
      };

      if self.cwd.join(&candidate).exists() {
        return candidate
          .strip_prefix(&self.cwd)
          .ok()
          .map(|p| p.to_path_buf());
      }
    }

    None
  }

  /// Check if a symbol is re-exported from a file
  fn is_re_exported(&self, file: &Path, symbol_name: &str) -> bool {
    if let Some(exports) = self.analyzer.exports.get(file) {
      exports.iter().any(|export| {
        export.local_name.as_deref() == Some(symbol_name)
          || (export.exported_name == symbol_name && export.local_name.is_none())
      })
    } else {
      false
    }
  }

  /// Compare two paths for equality (handling relative vs absolute)
  fn paths_equal(&self, path1: &Path, path2: &Path) -> bool {
    // Normalize both paths
    let p1 = if path1.is_absolute() {
      path1.strip_prefix(&self.cwd).unwrap_or(path1)
    } else {
      path1
    };

    let p2 = if path2.is_absolute() {
      path2.strip_prefix(&self.cwd).unwrap_or(path2)
    } else {
      path2
    };

    p1 == p2
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::profiler::Profiler;
  use crate::semantic::WorkspaceAnalyzer;
  use std::fs;
  use tempfile::TempDir;

  #[test]
  fn test_simple_resolve_appends_extensions() {
    // Test that simple_resolve appends extensions instead of replacing them
    // This is important for patterns like colors.css -> colors.css.ts

    // Create a temporary directory with test files
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let cwd = temp_dir.path();

    // Create a test file: libs/theme/colors.css.ts
    let theme_dir = cwd.join("libs").join("theme");
    fs::create_dir_all(&theme_dir).expect("Failed to create theme dir");
    let css_ts_file = theme_dir.join("colors.css.ts");
    fs::write(&css_ts_file, "export const red = '#ff0000';").expect("Failed to write test file");

    // Create analyzer and reference finder
    let profiler = Arc::new(Profiler::new(false));
    let analyzer = WorkspaceAnalyzer::new(vec![], cwd, profiler.clone()).expect("Failed to create analyzer");
    let reference_finder = ReferenceFinder::new(&analyzer, cwd, profiler);

    // Test: resolve "./colors.css" from libs/theme directory
    // Should find colors.css.ts by appending .ts
    let context = theme_dir.as_path();
    let specifier = "./colors.css";
    let resolved = reference_finder.simple_resolve(context, specifier);

    assert!(resolved.is_some(), "Expected to resolve colors.css to colors.css.ts");
    let resolved_path = resolved.unwrap();
    assert_eq!(
      resolved_path,
      PathBuf::from("libs/theme/colors.css.ts"),
      "Expected to resolve to colors.css.ts (extension appended)"
    );
  }

  #[test]
  fn test_simple_resolve_standard_extensions() {
    // Test that simple_resolve still works for standard TypeScript imports

    // Create a temporary directory with test files
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let cwd = temp_dir.path();

    // Create a test file: src/utils.ts
    let src_dir = cwd.join("src");
    fs::create_dir_all(&src_dir).expect("Failed to create src dir");
    let utils_file = src_dir.join("utils.ts");
    fs::write(&utils_file, "export function helper() {}").expect("Failed to write test file");

    // Create analyzer and reference finder
    let profiler = Arc::new(Profiler::new(false));
    let analyzer = WorkspaceAnalyzer::new(vec![], cwd, profiler.clone()).expect("Failed to create analyzer");
    let reference_finder = ReferenceFinder::new(&analyzer, cwd, profiler);

    // Test: resolve "./utils" from src directory
    // Should find utils.ts by appending .ts
    let context = src_dir.as_path();
    let specifier = "./utils";
    let resolved = reference_finder.simple_resolve(context, specifier);

    assert!(resolved.is_some(), "Expected to resolve utils to utils.ts");
    let resolved_path = resolved.unwrap();
    assert_eq!(
      resolved_path,
      PathBuf::from("src/utils.ts"),
      "Expected to resolve to utils.ts"
    );
  }

  #[test]
  fn test_simple_resolve_index_files() {
    // Test that simple_resolve can find index.ts files in directories

    // Create a temporary directory with test files
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let cwd = temp_dir.path();

    // Create a test file: src/components/index.ts
    let components_dir = cwd.join("src").join("components");
    fs::create_dir_all(&components_dir).expect("Failed to create components dir");
    let index_file = components_dir.join("index.ts");
    fs::write(&index_file, "export * from './Button';").expect("Failed to write test file");

    // Create analyzer and reference finder
    let profiler = Arc::new(Profiler::new(false));
    let analyzer = WorkspaceAnalyzer::new(vec![], cwd, profiler.clone()).expect("Failed to create analyzer");
    let reference_finder = ReferenceFinder::new(&analyzer, cwd, profiler);

    // Test: resolve "./components" from src directory
    // Should find components/index.ts
    let context = cwd.join("src");
    let specifier = "./components";
    let resolved = reference_finder.simple_resolve(context.as_path(), specifier);

    assert!(resolved.is_some(), "Expected to resolve components to components/index.ts");
    let resolved_path = resolved.unwrap();
    assert_eq!(
      resolved_path,
      PathBuf::from("src/components/index.ts"),
      "Expected to resolve to components/index.ts"
    );
  }
}
