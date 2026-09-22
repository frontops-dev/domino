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
  executor: Option<String>,
  /// `command` is sugar for the `nx:run-commands` executor, which matters when
  /// matching an executor-keyed `targetDefaults` entry.
  command: Option<serde_json::Value>,
  /// Kept as raw JSON: entries mix strings (`"production"`, `"^build"`) with
  /// several object shapes, and a strict type would fail the whole
  /// project.json on any shape it did not anticipate.
  inputs: Option<serde_json::Value>,
  #[serde(rename = "dependsOn")]
  depends_on: Option<serde_json::Value>,
}

impl NxTarget {
  fn effective_executor(&self) -> Option<&str> {
    self
      .executor
      .as_deref()
      .or_else(|| self.command.as_ref().map(|_| "nx:run-commands"))
  }
}

/// `nx.json` `targetDefaults`, keyed by target name, executor, or target-name
/// glob. Values are kept as raw JSON: a value is a target config object, or (in
/// newer Nx) an array of such objects.
type TargetDefaults = HashMap<String, serde_json::Value>;

#[derive(Debug, Deserialize)]
struct NxJsonTargetDefaults {
  #[serde(default, rename = "targetDefaults")]
  target_defaults: TargetDefaults,
}

/// Read `targetDefaults` from `<cwd>/nx.json`. Empty when the file or the key
/// is missing or unparsable: defaults only ever add links, so dropping them
/// degrades to the pre-targetDefaults behaviour rather than failing discovery.
fn read_target_defaults(cwd: &Path) -> TargetDefaults {
  fs::read_to_string(cwd.join("nx.json"))
    .ok()
    .and_then(|content| serde_json::from_str::<NxJsonTargetDefaults>(&content).ok())
    .map(|nx_json| nx_json.target_defaults)
    .unwrap_or_default()
}

/// Nx's `GLOB_CHARACTERS` (`packages/nx/src/utils/globs.ts`): a
/// `targetDefaults` key containing any of these is matched as a glob.
const NX_GLOB_CHARACTERS: [char; 7] = ['*', '|', '{', '}', '(', ')', '['];

/// Whether a glob `targetDefaults` key matches a target name the way Nx's
/// minimatch does: `*` stays within a `/` segment, `**` crosses segments, and
/// brace sets expand (Nx's own atomized-e2e key is `e2e-ci--**/*`). Extglob
/// forms such as `@(a|b)` are not supported and match nothing.
fn glob_key_matches(key: &str, target_name: &str) -> bool {
  globset::GlobBuilder::new(key)
    .literal_separator(true)
    .build()
    .map(|glob| glob.compile_matcher().is_match(target_name))
    .unwrap_or(false)
}

/// The `targetDefaults` entries that apply to a target, using Nx's key
/// precedence: the executor key (when the target uses that executor), then the
/// exact target name, then glob keys matching the target name, longest first
/// (ties broken by key so the order is deterministic).
///
/// Nx takes the first key with a matching entry and falls through a key whose
/// entries all fail their `filter`. Filters are not evaluated here, so a key
/// only stops the search when at least one of its entries is unfiltered; a
/// fully filtered key contributes its entries and the search continues. That
/// can over-select but never drops a link Nx would apply.
fn target_default_entries<'a>(
  target_name: &str,
  target: &NxTarget,
  defaults: &'a TargetDefaults,
) -> Vec<&'a serde_json::Value> {
  let mut glob_keys: Vec<&String> = defaults
    .keys()
    .filter(|key| key.contains(NX_GLOB_CHARACTERS) && glob_key_matches(key, target_name))
    .collect();
  glob_keys.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

  let candidates = target
    .effective_executor()
    .into_iter()
    .chain(std::iter::once(target_name))
    .chain(glob_keys.into_iter().map(String::as_str));

  let mut entries = Vec::new();
  for key in candidates {
    let Some(value) = defaults.get(key) else {
      continue;
    };
    let key_entries = default_entries(value);
    let key_definitely_applies = key_entries.iter().any(|e| e.get("filter").is_none());
    entries.extend(key_entries);
    if key_definitely_applies {
      break;
    }
  }
  entries
}

/// A `targetDefaults` value's entries: the object itself, or each object of the
/// newer array form. An entry's `filter` is not evaluated: every entry's links
/// are kept, so an unmatched filter can only over-select, never drop a link.
fn default_entries(value: &serde_json::Value) -> Vec<&serde_json::Value> {
  match value {
    serde_json::Value::Array(entries) => entries.iter().collect(),
    other => vec![other],
  }
}

/// Project names a list of `inputs` / `dependsOn` entries links to: the
/// `projects` field of object entries (`{ "input": "production", "projects":
/// ["x"] }`, `{ "projects": ["x"], "target": "build" }`). `projects` may be a
/// string or an array. Raw values are returned as written (including `self`,
/// `dependencies` and glob or `!` patterns); `core` interprets them.
///
/// `dependsOn` also accepts Nx's `"project:target"` string sugar. Nx reads the
/// part before a `:` as a project only when a project has that name, and
/// otherwise treats the whole string as a target name containing colons
/// (`build:prod`). Every `:`-prefix is returned as a candidate: one that is not
/// a project name never matches an affected project downstream, which is the
/// same rule. `^`-prefixed strings name the project's dependencies, not a
/// project, and `inputs` strings are named inputs, so neither yields one.
fn linked_projects(entries: &serde_json::Value, is_depends_on: bool) -> Vec<String> {
  let mut projects = Vec::new();
  for entry in entries.as_array().into_iter().flatten() {
    if let Some(sugar) = entry.as_str() {
      if is_depends_on && !sugar.starts_with('^') {
        projects.extend(
          sugar
            .match_indices(':')
            .map(|(colon, _)| sugar[..colon].to_string()),
        );
      }
      continue;
    }
    match entry.get("projects") {
      Some(serde_json::Value::String(s)) => projects.push(s.clone()),
      Some(serde_json::Value::Array(items)) => projects.extend(
        items
          .iter()
          .filter_map(|item| item.as_str())
          .map(str::to_string),
      ),
      _ => {}
    }
  }
  projects
}

/// Links declared by one target, with `targetDefaults` filling in whichever of
/// `inputs` / `dependsOn` the target does not set itself (a project's own
/// field replaces the default, as Nx merges them).
fn target_config_projects(
  target_name: &str,
  target: &NxTarget,
  defaults: &TargetDefaults,
) -> Vec<String> {
  let default = target_default_entries(target_name, target, defaults);

  let mut projects = Vec::new();
  for (own, key) in [
    (&target.inputs, "inputs"),
    (&target.depends_on, "dependsOn"),
  ] {
    match own {
      Some(value) => projects.extend(linked_projects(value, key == "dependsOn")),
      None => {
        for entry in &default {
          if let Some(value) = entry.get(key) {
            projects.extend(linked_projects(value, key == "dependsOn"));
          }
        }
      }
    }
  }
  projects
}

/// All project links declared across a project's targets, sorted and deduped.
/// `targetDefaults` apply only to targets the project declares.
fn config_dependencies(
  targets: Option<&HashMap<String, NxTarget>>,
  defaults: &TargetDefaults,
) -> Vec<String> {
  let mut projects: Vec<String> = targets
    .into_iter()
    .flatten()
    .flat_map(|(name, target)| target_config_projects(name, target, defaults))
    .collect();
  projects.sort();
  projects.dedup();
  projects
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
  let target_defaults = read_target_defaults(cwd);

  // Get projects from project.json files
  let project_json_projects = get_project_json_projects(cwd, &target_defaults)?;
  projects.extend(project_json_projects);

  // Get projects from workspace.json if it exists
  if let Ok(workspace_projects) = get_workspace_json_projects(cwd, &target_defaults) {
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

fn get_project_json_projects(cwd: &Path, target_defaults: &TargetDefaults) -> Result<Vec<Project>> {
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
    match parse_project_json(path, cwd, target_defaults) {
      Ok(project) => projects.push(project),
      Err(e) => warn!("Failed to parse project.json at {:?}: {}", path, e),
    }
  }

  Ok(projects)
}

fn parse_project_json(
  path: &Path,
  cwd: &Path,
  target_defaults: &TargetDefaults,
) -> Result<Project> {
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
    config_dependencies: config_dependencies(nx_project.targets.as_ref(), target_defaults),
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

fn get_workspace_json_projects(
  cwd: &Path,
  target_defaults: &TargetDefaults,
) -> Result<Vec<Project>> {
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
        config_dependencies: config_dependencies(nx_project.targets.as_ref(), target_defaults),
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

  #[test]
  fn test_glob_key_matches_like_minimatch() {
    assert!(glob_key_matches("e2e-ci--**/*", "e2e-ci--src/app.cy.ts"));
    assert!(glob_key_matches("e2e-ci--*", "e2e-ci--smoke"));
    // `*` stays within one segment
    assert!(!glob_key_matches("e2e-ci--*", "e2e-ci--src/app.cy.ts"));
    assert!(glob_key_matches("{build,test}", "test"));
    assert!(!glob_key_matches("{build,test}", "lint"));
  }

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
