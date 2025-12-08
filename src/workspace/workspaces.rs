use crate::error::{DominoError, Result};
use crate::types::Project;
use glob::glob;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

#[derive(Debug, Deserialize)]
struct PnpmWorkspace {
  packages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackageJson {
  name: String,
  workspaces: Option<Vec<String>>,
}

/// Check if the current directory is a generic workspace (npm/yarn/pnpm/bun)
pub fn is_workspace(cwd: &Path) -> bool {
  cwd.join("pnpm-workspace.yaml").exists() || has_npm_workspaces(cwd)
}

fn has_npm_workspaces(cwd: &Path) -> bool {
  let package_json_path = cwd.join("package.json");
  if !package_json_path.exists() {
    return false;
  }

  if let Ok(content) = fs::read_to_string(&package_json_path) {
    if let Ok(pkg_json) = serde_json::from_str::<PackageJson>(&content) {
      return pkg_json.workspaces.is_some();
    }
  }

  false
}

/// Get all workspace projects (npm/yarn/pnpm/bun)
pub fn get_projects(cwd: &Path) -> Result<Vec<Project>> {
  let workspace_patterns = get_workspace_patterns(cwd)?;

  let mut projects = Vec::new();

  for pattern in &workspace_patterns {
    // Skip negated patterns (starting with !)
    if pattern.starts_with('!') {
      continue;
    }

    let glob_pattern = cwd.join(pattern).join("package.json");
    let pattern_str = glob_pattern.to_string_lossy().to_string();

    for entry in glob(&pattern_str).map_err(|e| DominoError::Other(format!("Glob error: {}", e)))? {
      match entry {
        Ok(package_json_path) => {
          // Skip node_modules
          if package_json_path.to_string_lossy().contains("node_modules") {
            continue;
          }

          match parse_package_json(&package_json_path, cwd) {
            Ok(project) => projects.push(project),
            Err(e) => warn!(
              "Failed to parse package.json at {:?}: {}",
              package_json_path, e
            ),
          }
        }
        Err(e) => warn!("Error reading glob entry: {}", e),
      }
    }
  }

  debug!("Found {} workspace projects", projects.len());
  Ok(projects)
}

pub fn get_workspace_patterns(cwd: &Path) -> Result<Vec<String>> {
  // Try pnpm-workspace.yaml first
  let pnpm_workspace_path = cwd.join("pnpm-workspace.yaml");
  if pnpm_workspace_path.exists() {
    let content = fs::read_to_string(&pnpm_workspace_path)?;
    let workspace: PnpmWorkspace = serde_yaml::from_str(&content)
      .map_err(|e| DominoError::Parse(format!("Failed to parse pnpm-workspace.yaml: {}", e)))?;
    return Ok(workspace.packages);
  }

  // Try package.json workspaces
  let package_json_path = cwd.join("package.json");
  if package_json_path.exists() {
    let content = fs::read_to_string(&package_json_path)?;
    let pkg_json: PackageJson = serde_json::from_str(&content)
      .map_err(|e| DominoError::Parse(format!("Failed to parse package.json: {}", e)))?;

    if let Some(workspaces) = pkg_json.workspaces {
      return Ok(workspaces);
    }
  }

  Ok(vec![])
}

fn parse_package_json(path: &Path, cwd: &Path) -> Result<Project> {
  let content = fs::read_to_string(path)?;
  let pkg_json: PackageJson = serde_json::from_str(&content)
    .map_err(|e| DominoError::Parse(format!("Failed to parse package.json: {}", e)))?;

  let project_dir = path
    .parent()
    .ok_or_else(|| DominoError::Other("Invalid package path".to_string()))?;

  let source_root = if cwd.is_absolute() {
    project_dir.to_path_buf()
  } else {
    project_dir
      .strip_prefix(cwd)
      .unwrap_or(project_dir)
      .to_path_buf()
  };

  Ok(Project {
    name: pkg_json.name,
    source_root,
    ts_config: None,
    implicit_dependencies: vec![],
    targets: vec![],
  })
}
