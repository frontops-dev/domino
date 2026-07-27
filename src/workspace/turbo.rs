use crate::error::Result;
use crate::types::Project;

use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use super::workspaces;

/// Root config filenames Turborepo accepts, in resolution order.
/// `turbo.jsonc` was introduced in Turborepo v2.
const CONFIG_FILENAMES: &[&str] = &["turbo.json", "turbo.jsonc"];

/// The subset of a root `turbo.json` / `turbo.jsonc` that domino consumes.
///
/// Everything else is intentionally ignored, which is what makes both schema
/// generations work unchanged: Turborepo v1 puts its task map under `pipeline`
/// and v2 under `tasks`, and serde skips unknown keys. Per-task `inputs` /
/// `outputs`, `extends` chains in package-level configs, and remote cache
/// settings are out of scope.
#[derive(Debug, Default, Deserialize)]
pub struct TurboConfig {
  /// Glob patterns, relative to the workspace root, that feed Turborepo's
  /// global hash. A change to any of them invalidates every task in the
  /// workspace — the Turborepo analogue of Nx's `sharedGlobals`.
  #[serde(default, rename = "globalDependencies")]
  pub global_dependencies: Vec<String>,
}

/// Locate the root Turborepo config file, if there is one.
pub fn find_config_file(cwd: &Path) -> Option<PathBuf> {
  CONFIG_FILENAMES
    .iter()
    .map(|name| cwd.join(name))
    .find(|path| path.is_file())
}

/// Check if the current directory is a Turbo workspace
/// (Turbo-specific detection via turbo.json / turbo.jsonc)
pub fn is_turbo_workspace(cwd: &Path) -> bool {
  find_config_file(cwd).is_some()
}

/// Parse the root Turborepo config.
///
/// The file is JSONC (comments and trailing commas are allowed, and `.jsonc` is
/// explicitly supported by Turborepo v2), so both are stripped in place before
/// deserializing — the same treatment tsconfig files get. Stripping the whole
/// buffer up front (rather than streaming through a reader) is what makes
/// trailing-comma removal work, since that needs lookahead.
///
/// Returns `None` when no config exists, it can't be read, or it can't be
/// parsed. A malformed turbo.json degrades to "no global dependencies" (with a
/// warning) rather than failing the whole run.
pub fn parse_config(cwd: &Path) -> Option<TurboConfig> {
  let path = find_config_file(cwd)?;
  let mut content = match std::fs::read_to_string(&path) {
    Ok(content) => content,
    Err(e) => {
      warn!("Failed to read {}: {}", path.display(), e);
      return None;
    }
  };

  if let Err(e) = json_strip_comments::strip(&mut content) {
    warn!("Failed to strip comments from {}: {}", path.display(), e);
    return None;
  }

  match serde_json::from_str::<TurboConfig>(&content) {
    Ok(config) => {
      debug!(
        "Parsed {} ({} globalDependencies)",
        path.display(),
        config.global_dependencies.len()
      );
      Some(config)
    }
    Err(e) => {
      warn!("Failed to parse {}: {}", path.display(), e);
      None
    }
  }
}

/// Get all Turbo projects in the workspace
/// Delegates to the generic workspaces module for actual project discovery
/// (Turborepo reads workspaces from the root package.json / pnpm-workspace.yaml,
/// never from turbo.json).
pub fn get_projects(cwd: &Path) -> Result<Vec<Project>> {
  workspaces::get_projects(cwd)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  fn write_turbo(root: &Path, filename: &str, content: &str) {
    fs::write(root.join(filename), content).unwrap();
  }

  #[test]
  fn test_no_turbo_config_is_not_a_turbo_workspace() {
    let dir = TempDir::new().unwrap();
    assert!(!is_turbo_workspace(dir.path()));
    assert!(find_config_file(dir.path()).is_none());
    assert!(parse_config(dir.path()).is_none());
  }

  #[test]
  fn test_parse_v1_pipeline_layout() {
    let dir = TempDir::new().unwrap();
    write_turbo(
      dir.path(),
      "turbo.json",
      r#"{
  "$schema": "https://turbo.build/schema.json",
  "globalDependencies": [".env", "tsconfig.base.json"],
  "pipeline": {
    "build": { "dependsOn": ["^build"], "outputs": ["dist/**"] }
  }
}"#,
    );

    assert!(is_turbo_workspace(dir.path()));
    let config = parse_config(dir.path()).expect("v1 pipeline layout must parse");
    assert_eq!(
      config.global_dependencies,
      vec![".env".to_string(), "tsconfig.base.json".to_string()]
    );
  }

  #[test]
  fn test_parse_v2_tasks_layout() {
    let dir = TempDir::new().unwrap();
    write_turbo(
      dir.path(),
      "turbo.json",
      r#"{
  "globalDependencies": [".env"],
  "tasks": {
    "build": { "dependsOn": ["^build"], "inputs": ["src/**"] }
  },
  "remoteCache": { "enabled": true }
}"#,
    );

    let config = parse_config(dir.path()).expect("v2 tasks layout must parse");
    assert_eq!(config.global_dependencies, vec![".env".to_string()]);
  }

  #[test]
  fn test_parse_jsonc_comments_and_trailing_commas() {
    let dir = TempDir::new().unwrap();
    write_turbo(
      dir.path(),
      "turbo.json",
      r#"{
  // Global hash inputs
  "globalDependencies": [
    ".env", /* inline block comment */
    "config/*.json",
  ],
  "tasks": {}
}"#,
    );

    let config = parse_config(dir.path()).expect("JSONC turbo.json must parse");
    assert_eq!(
      config.global_dependencies,
      vec![".env".to_string(), "config/*.json".to_string()]
    );
  }

  #[test]
  fn test_turbo_jsonc_filename_supported() {
    let dir = TempDir::new().unwrap();
    write_turbo(
      dir.path(),
      "turbo.jsonc",
      r#"{
  // Turborepo v2 allows turbo.jsonc
  "globalDependencies": [".env"]
}"#,
    );

    assert!(is_turbo_workspace(dir.path()));
    assert_eq!(
      find_config_file(dir.path()),
      Some(dir.path().join("turbo.jsonc"))
    );
    let config = parse_config(dir.path()).expect("turbo.jsonc must parse");
    assert_eq!(config.global_dependencies, vec![".env".to_string()]);
  }

  #[test]
  fn test_turbo_json_takes_precedence_over_turbo_jsonc() {
    let dir = TempDir::new().unwrap();
    write_turbo(dir.path(), "turbo.json", r#"{"globalDependencies": ["a"]}"#);
    write_turbo(
      dir.path(),
      "turbo.jsonc",
      r#"{"globalDependencies": ["b"]}"#,
    );

    let config = parse_config(dir.path()).unwrap();
    assert_eq!(config.global_dependencies, vec!["a".to_string()]);
  }

  #[test]
  fn test_missing_global_dependencies_defaults_to_empty() {
    let dir = TempDir::new().unwrap();
    write_turbo(dir.path(), "turbo.json", r#"{"tasks": {"build": {}}}"#);

    let config = parse_config(dir.path()).expect("config without globalDependencies must parse");
    assert!(config.global_dependencies.is_empty());
  }

  #[test]
  fn test_malformed_json_degrades_gracefully() {
    let dir = TempDir::new().unwrap();
    write_turbo(dir.path(), "turbo.json", "{ not valid json at all ");

    // Still a Turbo workspace (the file exists), but no config is resolved.
    assert!(is_turbo_workspace(dir.path()));
    assert!(parse_config(dir.path()).is_none());
  }

  #[test]
  fn test_wrong_global_dependencies_type_degrades_gracefully() {
    let dir = TempDir::new().unwrap();
    write_turbo(
      dir.path(),
      "turbo.json",
      r#"{"globalDependencies": ".env"}"#,
    );

    assert!(parse_config(dir.path()).is_none());
  }

  #[test]
  fn test_empty_file_degrades_gracefully() {
    let dir = TempDir::new().unwrap();
    write_turbo(dir.path(), "turbo.json", "");

    assert!(parse_config(dir.path()).is_none());
  }
}
