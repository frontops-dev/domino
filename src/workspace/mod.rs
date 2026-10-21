pub mod nx;
pub mod rush;
pub mod turbo;
pub mod workspaces;

use crate::error::{DominoError, Result};
use crate::types::Project;
use glob::Pattern;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

const STATIC_EXCLUDES: &[&str] = &["!node_modules/", "!dist/", "!__fixtures__/"];

/// Build a directory walker that respects .gitignore, custom ignore files, and exclusion patterns.
pub(crate) fn build_walker(
  cwd: &Path,
  extra_excludes: &[&str],
  custom_ignore_files: &[&str],
) -> Result<ignore::Walk> {
  let mut overrides = OverrideBuilder::new(cwd);
  for pattern in STATIC_EXCLUDES.iter().chain(extra_excludes.iter()) {
    overrides
      .add(pattern)
      .map_err(|e| DominoError::Other(format!("Override error: {}", e)))?;
  }
  let overrides = overrides
    .build()
    .map_err(|e| DominoError::Other(format!("Override build error: {}", e)))?;

  let mut builder = WalkBuilder::new(cwd);
  builder
    .hidden(false)
    .git_ignore(true)
    .git_global(true)
    .git_exclude(true)
    .overrides(overrides);

  for filename in custom_ignore_files {
    builder.add_custom_ignore_filename(filename);
  }

  Ok(builder.build())
}

/// The workspace globs a `!` prefix excludes. `workspaces::get_projects` drops negated
/// patterns instead of applying them, which is invisible in a plain workspace repo but
/// would let this merge pull packages the workspace deliberately excludes into an Nx
/// project set, where they show up as affected. Matched with the same options as the
/// positive patterns, so `*` stops at a separator on both sides and a one-level exclusion
/// cannot swallow a nested member.
/// Nx reads a package-only project's configuration from the `nx` key of its `package.json`.
/// The generic workspace loader does not, so a merged project would arrive with no implicit
/// dependencies and a change to what it depends on would never mark it affected.
fn apply_package_nx_metadata(cwd: &Path, mut project: Project) -> Project {
  let manifest = cwd.join(&project.root).join("package.json");
  let Ok(contents) = std::fs::read_to_string(manifest) else {
    return project;
  };
  // Read the fields out of a Value rather than a typed manifest. Nx accepts several
  // shapes here (`tsConfig` is a string or an array, as `nx::deserialize_ts_config`
  // handles for project.json), and a typed parse fails whole: one unexpected shape would
  // discard the implicit dependencies and targets alongside it.
  let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&contents) else {
    return project;
  };
  let Some(nx) = manifest.get("nx") else {
    return project;
  };

  if project.implicit_dependencies.is_empty() {
    project.implicit_dependencies = nx
      .get("implicitDependencies")
      .and_then(serde_json::Value::as_array)
      .map(|deps| {
        deps
          .iter()
          .filter_map(serde_json::Value::as_str)
          .map(str::to_string)
          .collect()
      })
      .unwrap_or_default();
  }

  // A package can declare a source directory that is not the package root; the generic
  // loader only knows the root, and resolution aliases the package name through this.
  if let Some(source_root) = nx.get("sourceRoot").and_then(serde_json::Value::as_str) {
    project.source_root = PathBuf::from(source_root);
  }

  let targets = nx.get("targets").and_then(serde_json::Value::as_object);

  // The build target's `tsConfig` is how the resolver finds a `tsconfig.lib.json` that no
  // ancestor walk would reach, so it has to survive the merge alongside the names.
  if project.ts_config.is_none() {
    project.ts_config = targets
      .and_then(|targets| targets.get("build"))
      .and_then(|build| build.get("options"))
      .and_then(|options| options.get("tsConfig"))
      .and_then(first_ts_config)
      .map(|ts_config| cwd.join(ts_config));
  }

  if project.targets.is_empty() {
    project.targets = targets
      .map(|targets| targets.keys().cloned().collect())
      .unwrap_or_default();
  }
  project
}

/// `tsConfig` is a string, or an array whose first entry is the build config — the same
/// rule `nx::deserialize_ts_config` applies to `project.json`.
fn first_ts_config(value: &serde_json::Value) -> Option<String> {
  if let Some(ts_config) = value.as_str() {
    return Some(ts_config.to_string());
  }
  value.as_array()?.first()?.as_str().map(str::to_string)
}

fn excluded_workspace_patterns(cwd: &Path) -> Vec<Pattern> {
  workspaces::get_workspace_patterns(cwd)
    .unwrap_or_default()
    .iter()
    .filter_map(|pattern| pattern.strip_prefix('!'))
    .flat_map(|pattern| {
      // `!packages/examples/**` should exclude the directory itself as well as
      // everything under it.
      [pattern.trim_end_matches("/**"), pattern]
        .map(Pattern::new)
        .into_iter()
        .flatten()
    })
    .collect()
}

/// Detect workspace type and discover projects
pub fn discover_projects(cwd: &Path) -> Result<Vec<Project>> {
  // Try Nx first
  if nx::is_nx_workspace(cwd) {
    let mut projects = nx::get_projects(cwd)?;
    // Merge package-manager workspace members that have no project.json —
    // Nx itself infers these from package.json scripts, so an Nx workspace's
    // project set is a superset of its project.json files.
    if workspaces::is_workspace(cwd) {
      if let Ok(ws_projects) = workspaces::get_projects(cwd) {
        let known_roots: std::collections::HashSet<std::path::PathBuf> =
          projects.iter().map(|p| p.root.clone()).collect();
        let known_names: std::collections::HashSet<String> =
          projects.iter().map(|p| p.name.clone()).collect();
        let excluded = excluded_workspace_patterns(cwd);
        projects.extend(
          ws_projects
            .into_iter()
            .filter(|p| {
              !known_roots.contains(&p.root)
                && !known_names.contains(&p.name)
                && !excluded
                  .iter()
                  .any(|pattern| pattern.matches_path_with(&p.root, workspaces::GLOB_MATCH_OPTIONS))
            })
            .map(|p| apply_package_nx_metadata(cwd, p)),
        );
      }
    }
    return Ok(projects);
  }

  // Try Turbo (turbo.json)
  if turbo::is_turbo_workspace(cwd) {
    return turbo::get_projects(cwd);
  }

  // Try generic workspaces (npm/yarn/pnpm/bun)
  if workspaces::is_workspace(cwd) {
    return workspaces::get_projects(cwd);
  }

  // Try Rush (rush.json) — checked last to avoid interfering with existing workspace types
  if rush::is_rush_workspace(cwd) {
    return rush::get_projects(cwd);
  }

  // If none found, return empty
  Ok(vec![])
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::process::Command;
  use tempfile::TempDir;

  fn write_package_json(root: &Path, dir_name: &str, pkg_name: &str) {
    let dir = root.join(dir_name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
      dir.join("package.json"),
      format!(r#"{{ "name": "{}" }}"#, pkg_name),
    )
    .unwrap();
  }

  /// Merging workspace members into Nx discovery must not resurrect packages the
  /// workspace globs exclude: `workspaces::get_projects` drops `!` patterns rather than
  /// applying them, so the exclusion has to happen at the merge.
  #[test]
  fn test_nx_merge_honors_negated_workspace_patterns() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "name": "root", "workspaces": ["packages/*", "!packages/examples/**"] }"#,
    )
    .unwrap();
    write_package_json(root, "packages/lib", "lib");
    write_package_json(root, "packages/examples", "examples");

    let projects = discover_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert!(
      names.contains(&"lib"),
      "included package must be discovered"
    );
    assert!(
      !names.contains(&"examples"),
      "package excluded by a negated workspace glob must not be merged in, got {names:?}"
    );
  }

  /// A private monorepo root declares `workspaces` and no `name`. Requiring the name made
  /// the root parse fail, so no package-only project was merged at all.
  #[test]
  fn test_nx_merge_with_unnamed_root_manifest() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    )
    .unwrap();
    write_package_json(root, "packages/lib", "lib");

    let names: Vec<String> = discover_projects(root)
      .unwrap()
      .into_iter()
      .map(|p| p.name)
      .collect();
    assert!(
      names.iter().any(|name| name == "lib"),
      "an unnamed root must still be recognised as a workspace, got {names:?}"
    );
  }

  /// Nx infers a package-only project's config from the `nx` key of its package.json. The
  /// generic loader drops it, so the merged project would have no implicit dependencies
  /// and a change to what it depends on would never mark it affected.
  #[test]
  fn test_merged_package_keeps_nx_implicit_dependencies() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/web")).unwrap();
    fs::write(
      root.join("packages/web/package.json"),
      r#"{ "name": "web", "nx": { "implicitDependencies": ["shared-config"], "targets": { "build": {} } } }"#,
    )
    .unwrap();

    let projects = discover_projects(root).unwrap();
    let web = projects
      .iter()
      .find(|p| p.name == "web")
      .expect("web must be merged");

    assert_eq!(
      web.implicit_dependencies,
      vec!["shared-config".to_string()],
      "the package's nx.implicitDependencies must survive the merge"
    );
    assert_eq!(web.targets, vec!["build".to_string()]);
  }

  /// Yarn Classic's object form is a valid `workspaces` declaration. Accepting only the
  /// array made the root parse fail, so no package-only project was merged.
  #[test]
  fn test_nx_merge_with_yarn_object_workspaces() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "private": true, "workspaces": { "packages": ["packages/*"], "nohoist": ["**/react-native"] } }"#,
    )
    .unwrap();
    write_package_json(root, "packages/lib", "lib");

    let names: Vec<String> = discover_projects(root)
      .unwrap()
      .into_iter()
      .map(|p| p.name)
      .collect();
    assert!(
      names.iter().any(|name| name == "lib"),
      "Yarn's object form must be recognised as a workspace, got {names:?}"
    );
  }

  /// The build target's `tsConfig` is the only way the resolver reaches a project's
  /// `tsconfig.lib.json`, so it has to survive the merge, not just the target names.
  #[test]
  fn test_merged_package_keeps_build_ts_config() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/web")).unwrap();
    fs::write(
      root.join("packages/web/package.json"),
      r#"{ "name": "web", "nx": { "targets": { "build": { "options": { "tsConfig": "packages/web/tsconfig.app.json" } } } } }"#,
    )
    .unwrap();

    let projects = discover_projects(root).unwrap();
    let web = projects
      .iter()
      .find(|p| p.name == "web")
      .expect("web must be merged");

    assert_eq!(
      web.ts_config,
      Some(root.join("packages/web/tsconfig.app.json")),
      "the build target's tsConfig must survive the merge"
    );
  }

  /// `tsConfig` may be an array, and a typed parse of the whole manifest would fail on it
  /// — taking the implicit dependencies and targets down with it. Both must survive.
  #[test]
  fn test_merged_package_accepts_array_ts_config() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/web")).unwrap();
    fs::write(
      root.join("packages/web/package.json"),
      r#"{
        "name": "web",
        "nx": {
          "implicitDependencies": ["shared-config"],
          "targets": {
            "build": {
              "options": {
                "tsConfig": ["packages/web/tsconfig.app.json", "packages/web/tsconfig.spec.json"]
              }
            }
          }
        }
      }"#,
    )
    .unwrap();

    let projects = discover_projects(root).unwrap();
    let web = projects
      .iter()
      .find(|p| p.name == "web")
      .expect("web must be merged");

    assert_eq!(
      web.ts_config,
      Some(root.join("packages/web/tsconfig.app.json")),
      "the first entry of an array tsConfig is the build config"
    );
    assert_eq!(
      web.implicit_dependencies,
      vec!["shared-config".to_string()],
      "an array tsConfig must not discard the rest of the nx metadata"
    );
    assert_eq!(web.targets, vec!["build".to_string()]);
  }

  /// A package can put its sources somewhere other than the package root. Resolution
  /// aliases the package through `source_root`, so the generic loader's guess must not win
  /// over what the package declares.
  #[test]
  fn test_merged_package_keeps_nx_source_root() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "private": true, "workspaces": ["packages/*"] }"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("packages/foo/client")).unwrap();
    fs::write(
      root.join("packages/foo/package.json"),
      r#"{ "name": "foo", "nx": { "sourceRoot": "packages/foo/client" } }"#,
    )
    .unwrap();

    let projects = discover_projects(root).unwrap();
    let foo = projects
      .iter()
      .find(|p| p.name == "foo")
      .expect("foo must be merged");

    assert_eq!(
      foo.source_root,
      PathBuf::from("packages/foo/client"),
      "the package's declared sourceRoot must survive the merge"
    );
    assert_eq!(
      foo.root,
      PathBuf::from("packages/foo"),
      "the project root stays the package directory"
    );
  }

  /// A one-level exclusion must not swallow a nested member: `*` stops at a separator for
  /// the negated patterns exactly as it does for the positive ones.
  #[test]
  fn test_negated_pattern_does_not_cross_separators() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    Command::new("git")
      .args(["init", "-q"])
      .current_dir(root)
      .output()
      .unwrap();

    fs::write(root.join("nx.json"), "{}").unwrap();
    fs::write(
      root.join("package.json"),
      r#"{ "name": "root", "workspaces": ["packages/**", "!packages/*-example"] }"#,
    )
    .unwrap();
    write_package_json(root, "packages/top-example", "top-example");
    write_package_json(root, "packages/group/demo-example", "demo-example");

    let projects = discover_projects(root).unwrap();
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();

    assert!(
      !names.contains(&"top-example"),
      "the one-level exclusion covers packages/top-example, got {names:?}"
    );
    assert!(
      names.contains(&"demo-example"),
      "packages/group/demo-example is nested deeper than the exclusion, got {names:?}"
    );
  }
}
