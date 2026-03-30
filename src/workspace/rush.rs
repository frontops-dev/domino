use crate::error::{DominoError, Result};
use crate::types::Project;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RushJson {
  projects: Vec<RushProject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RushProject {
  package_name: String,
  project_folder: String,
}

#[derive(Debug, Deserialize)]
struct PackageJson {
  name: String,
}

/// Check if the current directory is a Rush workspace
pub fn is_rush_workspace(cwd: &Path) -> bool {
  cwd.join("rush.json").exists()
}

/// Get all Rush projects in the workspace
pub fn get_projects(cwd: &Path) -> Result<Vec<Project>> {
  let rush_json_path = cwd.join("rush.json");
  let content = fs::read_to_string(&rush_json_path)?;
  let rush_json: RushJson = serde_json::from_str(&content)
    .map_err(|e| DominoError::Parse(format!("Failed to parse rush.json: {}", e)))?;

  let mut projects = Vec::new();

  for rush_project in &rush_json.projects {
    let project_dir = cwd.join(&rush_project.project_folder);
    let package_json_path = project_dir.join("package.json");

    // Try to read the package.json for the canonical name, fall back to rush.json packageName
    let name = if package_json_path.exists() {
      match fs::read_to_string(&package_json_path) {
        Ok(pkg_content) => match serde_json::from_str::<PackageJson>(&pkg_content) {
          Ok(pkg) => pkg.name,
          Err(e) => {
            warn!(
              "Failed to parse package.json at {:?}: {}, using rush.json packageName",
              package_json_path, e
            );
            rush_project.package_name.clone()
          }
        },
        Err(e) => {
          warn!(
            "Failed to read package.json at {:?}: {}, using rush.json packageName",
            package_json_path, e
          );
          rush_project.package_name.clone()
        }
      }
    } else {
      warn!(
        "package.json not found at {:?}, using rush.json packageName",
        package_json_path
      );
      rush_project.package_name.clone()
    };

    let source_root = Path::new(&rush_project.project_folder).to_path_buf();

    projects.push(Project {
      name,
      root: source_root.clone(),
      source_root,
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    });
  }

  debug!("Found {} Rush projects", projects.len());
  Ok(projects)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  fn create_rush_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    // Create rush.json
    fs::write(
      root.join("rush.json"),
      r#"{
  "$schema": "https://developer.microsoft.com/json-schemas/rush/v5/rush.schema.json",
  "rushVersion": "5.102.0",
  "pnpmVersion": "8.7.0",
  "projects": [
    {
      "packageName": "@myorg/app-service",
      "projectFolder": "services/app",
      "tags": ["service"]
    },
    {
      "packageName": "@myorg/shared-lib",
      "projectFolder": "packages/shared",
      "tags": ["package"]
    }
  ]
}"#,
    )
    .unwrap();

    // Create project directories with package.json
    fs::create_dir_all(root.join("services/app")).unwrap();
    fs::write(
      root.join("services/app/package.json"),
      r#"{ "name": "@myorg/app-service", "version": "1.0.0" }"#,
    )
    .unwrap();

    fs::create_dir_all(root.join("packages/shared")).unwrap();
    fs::write(
      root.join("packages/shared/package.json"),
      r#"{ "name": "@myorg/shared-lib", "version": "1.0.0" }"#,
    )
    .unwrap();

    dir
  }

  #[test]
  fn test_is_rush_workspace() {
    let dir = create_rush_fixture();
    assert!(is_rush_workspace(dir.path()));
  }

  #[test]
  fn test_is_not_rush_workspace() {
    let dir = TempDir::new().unwrap();
    assert!(!is_rush_workspace(dir.path()));
  }

  #[test]
  fn test_get_projects() {
    let dir = create_rush_fixture();
    let projects = get_projects(dir.path()).unwrap();

    assert_eq!(projects.len(), 2);

    assert_eq!(projects[0].name, "@myorg/app-service");
    assert_eq!(
      projects[0].source_root,
      Path::new("services/app").to_path_buf()
    );

    assert_eq!(projects[1].name, "@myorg/shared-lib");
    assert_eq!(
      projects[1].source_root,
      Path::new("packages/shared").to_path_buf()
    );
  }

  #[test]
  fn test_get_projects_missing_package_json_falls_back() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::write(
      root.join("rush.json"),
      r#"{
  "rushVersion": "5.102.0",
  "projects": [
    {
      "packageName": "@myorg/missing-pkg",
      "projectFolder": "packages/missing"
    }
  ]
}"#,
    )
    .unwrap();

    fs::create_dir_all(root.join("packages/missing")).unwrap();
    // No package.json created

    let projects = get_projects(root).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "@myorg/missing-pkg");
  }
}
