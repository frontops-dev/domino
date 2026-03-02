use crate::error::{DominoError, Result};
use crate::types::Project;
use glob::{MatchOptions, Pattern};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

const GLOB_MATCH_OPTIONS: MatchOptions = MatchOptions {
  case_sensitive: true,
  require_literal_separator: true,
  require_literal_leading_dot: false,
};

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

  let glob_patterns: Vec<Pattern> = workspace_patterns
    .iter()
    .filter(|p| !p.starts_with('!'))
    .filter_map(|p| Pattern::new(&format!("{}/package.json", p)).ok())
    .collect();

  if glob_patterns.is_empty() {
    return Ok(vec![]);
  }

  let walker = super::build_walker(cwd, &[], &[])?;

  let mut projects = Vec::new();
  for entry in walker {
    let entry = match entry {
      Ok(e) => e,
      Err(e) => {
        warn!("Walk error during workspace discovery: {}", e);
        continue;
      }
    };
    if entry.file_type().is_none_or(|ft| !ft.is_file()) || entry.file_name() != "package.json" {
      continue;
    }

    let relative = match entry.path().strip_prefix(cwd) {
      Ok(rel) => rel,
      Err(_) => continue,
    };

    if !glob_patterns
      .iter()
      .any(|p| p.matches_path_with(relative, GLOB_MATCH_OPTIONS))
    {
      continue;
    }

    match parse_package_json(entry.path(), cwd) {
      Ok(project) => projects.push(project),
      Err(e) => warn!("Failed to parse package.json at {:?}: {}", entry.path(), e),
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::process::Command;
  use tempfile::TempDir;

  fn create_workspace_fixture(patterns: &[&str]) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    let workspaces_json: Vec<String> = patterns.iter().map(|p| format!(r#""{}""#, p)).collect();
    fs::write(
      root.join("package.json"),
      format!(
        r#"{{ "name": "root", "workspaces": [{}] }}"#,
        workspaces_json.join(", ")
      ),
    )
    .unwrap();

    dir
  }

  fn write_package_json(root: &std::path::Path, dir_name: &str, pkg_name: &str) {
    let dir = root.join(dir_name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
      dir.join("package.json"),
      format!(r#"{{ "name": "{}" }}"#, pkg_name),
    )
    .unwrap();
  }

  #[test]
  fn test_valid_workspace_projects_discovered() {
    let dir = create_workspace_fixture(&["packages/*"]);
    let root = dir.path();

    write_package_json(root, "packages/utils", "@myorg/utils");
    write_package_json(root, "packages/core", "@myorg/core");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 2);
    assert!(names.contains(&"@myorg/utils"));
    assert!(names.contains(&"@myorg/core"));
  }

  #[test]
  fn test_gitignore_excludes_workspace_project() {
    let dir = create_workspace_fixture(&["packages/*"]);
    let root = dir.path();

    write_package_json(root, "packages/utils", "@myorg/utils");
    write_package_json(root, "packages/ignored-pkg", "@myorg/ignored-pkg");

    fs::write(root.join(".gitignore"), "packages/ignored-pkg\n").unwrap();

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"@myorg/utils"));
    assert!(!names.contains(&"@myorg/ignored-pkg"));
  }

  #[test]
  fn test_dist_excluded_by_default() {
    let dir = create_workspace_fixture(&["packages/*", "dist/*"]);
    let root = dir.path();

    write_package_json(root, "packages/utils", "@myorg/utils");
    write_package_json(root, "dist/utils", "@myorg/utils-dist");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"@myorg/utils"));
    assert!(!names.contains(&"@myorg/utils-dist"));
  }

  #[test]
  fn test_pnpm_workspace_discovery() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(
      root.join("pnpm-workspace.yaml"),
      "packages:\n  - 'apps/*'\n  - 'libs/*'\n",
    )
    .unwrap();

    write_package_json(root, "apps/web", "@myorg/web");
    write_package_json(root, "libs/shared", "@myorg/shared");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 2);
    assert!(names.contains(&"@myorg/web"));
    assert!(names.contains(&"@myorg/shared"));
  }

  #[test]
  fn test_non_matching_packages_excluded() {
    let dir = create_workspace_fixture(&["packages/*"]);
    let root = dir.path();

    write_package_json(root, "packages/utils", "@myorg/utils");
    write_package_json(root, "other/stray", "@myorg/stray");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"@myorg/utils"));
  }

  #[test]
  fn test_node_modules_excluded_by_default() {
    let dir = create_workspace_fixture(&["packages/*"]);
    let root = dir.path();

    write_package_json(root, "packages/utils", "@myorg/utils");
    write_package_json(root, "node_modules/some-dep", "@some/dep");
    write_package_json(
      root,
      "packages/utils/node_modules/nested-dep",
      "@nested/dep",
    );

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"@myorg/utils"));
  }

  #[test]
  fn test_star_does_not_match_across_separators() {
    let dir = create_workspace_fixture(&["packages/*"]);
    let root = dir.path();

    write_package_json(root, "packages/utils", "@myorg/utils");
    write_package_json(root, "packages/utils/nested/deep", "@myorg/deep");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"@myorg/utils"));
  }

  #[test]
  fn test_source_root_with_absolute_cwd() {
    let dir = create_workspace_fixture(&["packages/*"]);
    let root = dir.path();

    write_package_json(root, "packages/utils", "@myorg/utils");

    let projects = get_projects(root).unwrap();

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].source_root, root.join("packages/utils"));
  }
}
