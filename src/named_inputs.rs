use crate::types::{ChangedFile, Project};
use glob::Pattern;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Resolved named inputs configuration from nx.json
#[derive(Debug, Default)]
pub struct ResolvedNamedInputs {
  /// Glob patterns for workspace-root files that invalidate all projects
  /// e.g., "babel.config.json", "patches/*"
  pub global_patterns: Vec<Pattern>,
  /// Pre-compiled negation glob patterns for project-root files to exclude
  /// e.g., "**/*.figma.tsx"
  pub negation_patterns: Vec<Pattern>,
}

#[derive(Debug, Deserialize)]
struct NxJson {
  #[serde(default, rename = "namedInputs")]
  named_inputs: HashMap<String, Vec<serde_json::Value>>,
}

/// Parse and resolve namedInputs from nx.json.
/// Returns None if nx.json doesn't exist or has no namedInputs.
pub fn resolve_from_nx_json(cwd: &Path) -> Option<ResolvedNamedInputs> {
  let nx_json_path = cwd.join("nx.json");
  let content = fs::read_to_string(&nx_json_path).ok()?;
  let nx_json: NxJson = serde_json::from_str(&content).ok()?;

  if nx_json.named_inputs.is_empty() {
    debug!("No namedInputs found in nx.json");
    return None;
  }

  debug!(
    "Found {} named inputs in nx.json",
    nx_json.named_inputs.len()
  );

  // Resolve the "default" named input recursively
  let mut resolved_patterns = Vec::new();
  let mut visited = std::collections::HashSet::new();
  resolve_named_input(
    "default",
    &nx_json.named_inputs,
    &mut resolved_patterns,
    &mut visited,
  );

  if resolved_patterns.is_empty() {
    debug!("No patterns resolved from namedInputs.default");
    return None;
  }

  let mut global_patterns = Vec::new();
  let mut negation_patterns = Vec::new();

  for pattern_str in &resolved_patterns {
    if let Some(negated) = pattern_str.strip_prefix('!') {
      // Negation pattern
      if let Some(suffix) = negated.strip_prefix("{projectRoot}/") {
        match Pattern::new(suffix) {
          Ok(pat) => {
            debug!("Negation pattern (project-root): !{}", suffix);
            negation_patterns.push(pat);
          }
          Err(e) => {
            warn!("Invalid negation glob pattern '{}': {}", suffix, e);
          }
        }
      } else if let Some(suffix) = negated.strip_prefix("{workspaceRoot}/") {
        debug!(
          "Negation pattern (workspace-root): !{} — skipping (not yet supported)",
          suffix
        );
      }
    } else if let Some(suffix) = pattern_str.strip_prefix("{workspaceRoot}/") {
      // Global workspace-root pattern
      match Pattern::new(suffix) {
        Ok(pat) => {
          debug!("Global pattern: {}", suffix);
          global_patterns.push(pat);
        }
        Err(e) => {
          warn!("Invalid glob pattern '{}': {}", suffix, e);
        }
      }
    }
    // {projectRoot}/** positive patterns are already handled by sourceRoot-based ownership
  }

  if global_patterns.is_empty() && negation_patterns.is_empty() {
    debug!("No actionable patterns resolved from namedInputs");
    return None;
  }

  debug!(
    "Resolved {} global patterns, {} negation patterns",
    global_patterns.len(),
    negation_patterns.len()
  );

  Some(ResolvedNamedInputs {
    global_patterns,
    negation_patterns,
  })
}

/// Recursively resolve a named input, following references to other named inputs.
fn resolve_named_input(
  name: &str,
  all_inputs: &HashMap<String, Vec<serde_json::Value>>,
  resolved: &mut Vec<String>,
  visited: &mut std::collections::HashSet<String>,
) {
  if !visited.insert(name.to_string()) {
    debug!("Circular reference detected in namedInputs: {}", name);
    return;
  }

  let entries = match all_inputs.get(name) {
    Some(entries) => entries,
    None => {
      debug!("Named input '{}' not found in nx.json", name);
      return;
    }
  };

  for entry in entries {
    match entry {
      serde_json::Value::String(s) => {
        if s.starts_with('{') || s.starts_with('!') {
          // It's a file pattern (e.g., "{projectRoot}/**/*" or "!{projectRoot}/**/*.spec.ts")
          resolved.push(s.clone());
        } else {
          // It's a reference to another named input (e.g., "sharedGlobals")
          resolve_named_input(s, all_inputs, resolved, visited);
        }
      }
      serde_json::Value::Object(_) => {
        // Object entries like {"runtime": "node"} or {"externalDependencies": [...]}
        // These are not file patterns — skip them
        debug!("Skipping object entry in namedInput '{}'", name);
      }
      _ => {
        debug!("Skipping unexpected entry type in namedInput '{}'", name);
      }
    }
  }
}

impl ResolvedNamedInputs {
  /// Check if a changed file matches any global invalidation pattern.
  /// `file_path` should be relative to workspace root.
  pub fn matches_global_pattern(&self, file_path: &Path) -> bool {
    let path_str = match file_path.to_str() {
      Some(s) => s,
      None => return false,
    };

    let opts = glob::MatchOptions {
      case_sensitive: true,
      require_literal_separator: false,
      require_literal_leading_dot: false,
    };

    for pattern in &self.global_patterns {
      if pattern.matches_with(path_str, opts) {
        debug!(
          "File '{}' matches global pattern '{}'",
          path_str,
          pattern.as_str()
        );
        return true;
      }
    }
    false
  }

  /// Check if a changed file should be excluded by negation patterns.
  /// `file_path` should be relative to workspace root.
  /// `project_root` is the project's root directory (relative to workspace root).
  pub fn is_negated(&self, file_path: &Path, project_root: &Path) -> bool {
    if self.negation_patterns.is_empty() {
      return false;
    }

    // Check if file is under this project root
    let relative = match file_path.strip_prefix(project_root) {
      Ok(rel) => rel,
      Err(_) => return false,
    };

    let relative_str = match relative.to_str() {
      Some(s) => s,
      None => return false,
    };

    let opts = glob::MatchOptions {
      case_sensitive: true,
      require_literal_separator: false,
      require_literal_leading_dot: false,
    };

    for pat in &self.negation_patterns {
      if pat.matches_with(relative_str, opts) {
        debug!(
          "File '{}' excluded by negation pattern '!{{projectRoot}}/{}'",
          file_path.display(),
          pat.as_str()
        );
        return true;
      }
    }
    false
  }

  /// Check if a file is negated by any of the given project roots.
  /// Returns true if the file matches a negation pattern for any project that owns it.
  pub fn is_negated_by_any_project(&self, file_path: &Path, project_roots: &[&Path]) -> bool {
    if self.negation_patterns.is_empty() {
      return false;
    }
    project_roots
      .iter()
      .any(|root| self.is_negated(file_path, root))
  }
}

/// Check if any changed file triggers global invalidation.
/// Returns `Some(file_path)` of the triggering file, or `None`.
pub fn check_global_invalidation(
  inputs: &ResolvedNamedInputs,
  changed_files: &[ChangedFile],
) -> Option<PathBuf> {
  for changed_file in changed_files {
    if inputs.matches_global_pattern(&changed_file.file_path) {
      debug!(
        "Global invalidation triggered by {:?}",
        changed_file.file_path
      );
      return Some(changed_file.file_path.clone());
    }
  }
  None
}

/// Filter out changed files that match negation patterns from namedInputs.
/// Returns a new vector with negated files removed.
pub fn filter_negated_files(
  inputs: &ResolvedNamedInputs,
  changed_files: Vec<ChangedFile>,
  projects: &[Project],
) -> Vec<ChangedFile> {
  if inputs.negation_patterns.is_empty() {
    return changed_files;
  }

  let project_roots: Vec<&Path> = projects.iter().map(|p| p.root.as_path()).collect();

  let before = changed_files.len();
  let filtered: Vec<ChangedFile> = changed_files
    .into_iter()
    .filter(|f| !inputs.is_negated_by_any_project(&f.file_path, &project_roots))
    .collect();
  let after = filtered.len();

  if before != after {
    debug!(
      "Filtered {} files by namedInputs negation patterns ({} → {})",
      before - after,
      before,
      after
    );
  }
  filtered
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn write_nx_json(root: &Path, content: &str) {
    fs::write(root.join("nx.json"), content).unwrap();
  }

  #[test]
  fn test_resolve_global_patterns() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["{projectRoot}/**/*", "sharedGlobals"],
          "sharedGlobals": [
            "{workspaceRoot}/babel.config.json",
            "{workspaceRoot}/patches/*"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 2);
    assert!(resolved.matches_global_pattern(&PathBuf::from("babel.config.json")));
    assert!(resolved.matches_global_pattern(&PathBuf::from("patches/some-patch.patch")));
    assert!(!resolved.matches_global_pattern(&PathBuf::from("src/index.ts")));
  }

  #[test]
  fn test_resolve_negation_patterns() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": [
            "{projectRoot}/**/*",
            "!{projectRoot}/**/*.figma.tsx",
            "!{projectRoot}/**/*.stories.tsx"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.negation_patterns.len(), 2);

    let project_root = PathBuf::from("libs/ui");
    assert!(resolved.is_negated(
      &PathBuf::from("libs/ui/src/Button.figma.tsx"),
      &project_root
    ));
    assert!(resolved.is_negated(
      &PathBuf::from("libs/ui/src/Button.stories.tsx"),
      &project_root
    ));
    assert!(!resolved.is_negated(&PathBuf::from("libs/ui/src/Button.tsx"), &project_root));
  }

  #[test]
  fn test_recursive_resolution() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["{projectRoot}/**/*", "sharedGlobals"],
          "sharedGlobals": ["{workspaceRoot}/babel.config.json", "ciInputs"],
          "ciInputs": ["{workspaceRoot}/ci/utils.Jenkinsfile"]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 2);
    assert!(resolved.matches_global_pattern(&PathBuf::from("babel.config.json")));
    assert!(resolved.matches_global_pattern(&PathBuf::from("ci/utils.Jenkinsfile")));
  }

  #[test]
  fn test_circular_reference_handled() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["a"],
          "a": ["b"],
          "b": ["a", "{workspaceRoot}/file.json"]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 1);
    assert!(resolved.matches_global_pattern(&PathBuf::from("file.json")));
  }

  #[test]
  fn test_object_entries_skipped() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": [
            "{projectRoot}/**/*",
            {"runtime": "node"},
            "{workspaceRoot}/global.json"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 1);
  }

  #[test]
  fn test_no_named_inputs_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(root, r#"{"npmScope": "myorg"}"#);

    assert!(resolve_from_nx_json(root).is_none());
  }

  #[test]
  fn test_no_default_named_input_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "production": ["{projectRoot}/**/*"]
        }
      }"#,
    );

    // No "default" key → no patterns resolved
    assert!(resolve_from_nx_json(root).is_none());
  }

  #[test]
  fn test_is_negated_by_any_project() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": [
            "{projectRoot}/**/*",
            "!{projectRoot}/**/*.spec.ts"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();

    let root1 = PathBuf::from("libs/a");
    let root2 = PathBuf::from("libs/b");
    let roots: Vec<&Path> = vec![root1.as_path(), root2.as_path()];

    assert!(resolved.is_negated_by_any_project(&PathBuf::from("libs/a/src/foo.spec.ts"), &roots));
    assert!(!resolved.is_negated_by_any_project(&PathBuf::from("libs/a/src/foo.ts"), &roots));
  }
}
