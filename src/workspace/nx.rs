use crate::error::{DominoError, Result};
use crate::types::Project;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NxProjectJson {
  name: Option<String>,
  root: Option<String>,
  source_root: Option<String>,
  project_type: Option<String>,
  #[serde(default)]
  implicit_dependencies: Vec<String>,
  targets: Option<HashMap<String, NxTarget>>,
}

#[derive(Debug, Deserialize)]
struct NxTarget {
  options: Option<NxTargetOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NxTargetOptions {
  #[serde(default, deserialize_with = "deserialize_ts_config")]
  ts_config: Option<String>,
}

fn deserialize_ts_config<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  use serde::de::Deserialize;

  #[derive(Deserialize)]
  #[serde(untagged)]
  enum StringOrVec {
    String(String),
    Vec(Vec<String>),
  }

  match Option::<StringOrVec>::deserialize(deserializer)? {
    Some(StringOrVec::String(s)) => Ok(Some(s)),
    Some(StringOrVec::Vec(v)) => Ok(v.first().cloned()),
    None => Ok(None),
  }
}

#[derive(Debug, Deserialize)]
struct WorkspaceJson {
  projects: HashMap<String, serde_json::Value>,
}

/// Check if the current directory is an Nx workspace
pub fn is_nx_workspace(cwd: &Path) -> bool {
  cwd.join("nx.json").exists()
}

/// Get all Nx projects in the workspace
pub fn get_projects(cwd: &Path) -> Result<Vec<Project>> {
  let mut projects = Vec::new();

  // Get projects from project.json files
  let project_json_projects = get_project_json_projects(cwd)?;
  projects.extend(project_json_projects);

  // Get projects from workspace.json if it exists
  if let Ok(workspace_projects) = get_workspace_json_projects(cwd) {
    // Filter out duplicates (prefer project.json)
    let existing_names: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();

    for project in workspace_projects {
      if !existing_names.contains(&project.name) {
        projects.push(project);
      }
    }
  }

  debug!("Found {} Nx projects", projects.len());
  Ok(projects)
}

fn get_project_json_projects(cwd: &Path) -> Result<Vec<Project>> {
  let walker = super::build_walker(cwd, &[], &[".nxignore"])?;

  let mut projects = Vec::new();
  for entry in walker {
    let entry = match entry {
      Ok(e) => e,
      Err(e) => {
        warn!("Walk error during project discovery: {}", e);
        continue;
      }
    };
    if entry.file_type().is_none_or(|ft| !ft.is_file()) || entry.file_name() != "project.json" {
      continue;
    }

    let path = entry.path();
    match parse_project_json(path, cwd) {
      Ok(project) => projects.push(project),
      Err(e) => warn!("Failed to parse project.json at {:?}: {}", path, e),
    }
  }

  Ok(projects)
}

fn parse_project_json(path: &Path, cwd: &Path) -> Result<Project> {
  let content = fs::read_to_string(path).map_err(DominoError::Io)?;

  let nx_project: NxProjectJson = serde_json::from_str(&content)
    .map_err(|e| DominoError::Parse(format!("Failed to parse project.json: {}", e)))?;

  let project_dir = path
    .parent()
    .ok_or_else(|| DominoError::Other("Invalid project path".to_string()))?;

  let name = nx_project.name.clone().unwrap_or_else(|| {
    project_dir
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("unknown")
      .to_string()
  });

  let source_root = if let Some(ref sr) = nx_project.source_root {
    PathBuf::from(sr)
  } else {
    project_dir
      .strip_prefix(cwd)
      .unwrap_or(project_dir)
      .to_path_buf()
  };

  let ts_config = resolve_tsconfig(&nx_project, project_dir, &source_root, cwd);

  let targets: Vec<String> = nx_project
    .targets
    .as_ref()
    .map(|t| t.keys().cloned().collect())
    .unwrap_or_default();

  let root = project_dir
    .strip_prefix(cwd)
    .unwrap_or(project_dir)
    .to_path_buf();

  Ok(Project {
    name,
    root,
    source_root,
    ts_config,
    implicit_dependencies: nx_project.implicit_dependencies,
    targets,
  })
}

fn resolve_tsconfig(
  nx_project: &NxProjectJson,
  project_dir: &Path,
  source_root: &Path,
  cwd: &Path,
) -> Option<PathBuf> {
  // Check if tsConfig is specified in build target
  if let Some(targets) = &nx_project.targets {
    if let Some(build) = targets.get("build") {
      if let Some(options) = &build.options {
        if let Some(ts_config) = &options.ts_config {
          return Some(cwd.join(ts_config));
        }
      }
    }
  }

  // Determine project root
  let project_root = if source_root.exists() {
    source_root.parent().unwrap_or(project_dir)
  } else {
    project_dir
  };

  // Try different tsconfig patterns
  let project_type = nx_project.project_type.as_deref().unwrap_or("");

  let tsconfig_name = if project_type == "library" {
    "tsconfig.lib.json"
  } else {
    "tsconfig.app.json"
  };

  let ts_config_path = project_root.join(tsconfig_name);

  if ts_config_path.exists() {
    Some(ts_config_path)
  } else {
    // Fallback to tsconfig.json
    let fallback = project_root.join("tsconfig.json");
    if fallback.exists() {
      Some(fallback)
    } else {
      None
    }
  }
}

fn get_workspace_json_projects(cwd: &Path) -> Result<Vec<Project>> {
  let workspace_path = cwd.join("workspace.json");

  if !workspace_path.exists() {
    return Ok(vec![]);
  }

  let content = fs::read_to_string(&workspace_path)?;
  let workspace: WorkspaceJson = serde_json::from_str(&content)
    .map_err(|e| DominoError::Parse(format!("Failed to parse workspace.json: {}", e)))?;

  let mut projects = Vec::new();

  for (name, value) in workspace.projects {
    if let Ok(nx_project) = serde_json::from_value::<NxProjectJson>(value) {
      let source_root = if let Some(ref sr) = nx_project.source_root {
        PathBuf::from(sr)
      } else {
        PathBuf::from(&name)
      };

      let ts_config = resolve_tsconfig(&nx_project, cwd, &source_root, cwd);

      let targets: Vec<String> = nx_project
        .targets
        .as_ref()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default();

      // Use the parsed root property if available, otherwise fall back to the key name
      let root = nx_project
        .root
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&name));

      projects.push(Project {
        name,
        root,
        source_root,
        ts_config,
        implicit_dependencies: nx_project.implicit_dependencies,
        targets,
      });
    }
  }

  Ok(projects)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::process::Command;
  use tempfile::TempDir;

  fn create_nx_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), r#"{"npmScope": "myorg"}"#).unwrap();

    dir
  }

  fn write_project_json(root: &Path, dir_name: &str, project_name: &str) {
    let dir = root.join(dir_name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
      dir.join("project.json"),
      format!(r#"{{ "name": "{}" }}"#, project_name),
    )
    .unwrap();
  }

  #[test]
  fn test_valid_projects_discovered() {
    let dir = create_nx_fixture();
    let root = dir.path();

    write_project_json(root, "libs/shared", "shared");
    write_project_json(root, "apps/web", "web");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 2);
    assert!(names.contains(&"shared"));
    assert!(names.contains(&"web"));
  }

  #[test]
  fn test_gitignore_excludes_project() {
    let dir = create_nx_fixture();
    let root = dir.path();

    write_project_json(root, "libs/shared", "shared");
    write_project_json(root, "libs/ignored-lib", "ignored-lib");

    fs::write(root.join(".gitignore"), "libs/ignored-lib\n").unwrap();

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"shared"));
    assert!(!names.contains(&"ignored-lib"));
  }

  #[test]
  fn test_nxignore_excludes_project() {
    let dir = create_nx_fixture();
    let root = dir.path();

    write_project_json(root, "libs/shared", "shared");
    write_project_json(root, "libs/nx-ignored", "nx-ignored");

    fs::write(root.join(".nxignore"), "libs/nx-ignored\n").unwrap();

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"shared"));
    assert!(!names.contains(&"nx-ignored"));
  }

  #[test]
  fn test_dist_excluded_by_default() {
    let dir = create_nx_fixture();
    let root = dir.path();

    write_project_json(root, "libs/shared", "shared");
    write_project_json(root, "dist/libs/shared", "shared-dist");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"shared"));
    assert!(!names.contains(&"shared-dist"));
  }

  #[test]
  fn test_fixtures_excluded_by_default() {
    let dir = create_nx_fixture();
    let root = dir.path();

    write_project_json(root, "libs/shared", "shared");
    write_project_json(root, "__fixtures__/mock-project", "mock");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"shared"));
    assert!(!names.contains(&"mock"));
  }

  #[test]
  fn test_node_modules_excluded_by_default() {
    let dir = create_nx_fixture();
    let root = dir.path();

    write_project_json(root, "libs/shared", "shared");
    write_project_json(root, "node_modules/some-dep", "some-dep");
    write_project_json(root, "libs/shared/node_modules/nested-dep", "nested-dep");

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"shared"));
  }

  #[test]
  fn test_nested_gitignore_respected() {
    let dir = create_nx_fixture();
    let root = dir.path();

    write_project_json(root, "libs/shared", "shared");
    write_project_json(root, "libs/generated/auto-gen", "auto-gen");

    let libs_dir = root.join("libs/generated");
    fs::create_dir_all(&libs_dir).unwrap();
    fs::write(libs_dir.join(".gitignore"), "auto-gen\n").unwrap();

    let projects = get_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert_eq!(projects.len(), 1);
    assert!(names.contains(&"shared"));
    assert!(!names.contains(&"auto-gen"));
  }
}
