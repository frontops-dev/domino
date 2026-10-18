use crate::types::Project;
use json_strip_comments::StripComments;
use oxc_resolver::{AliasValue, ResolveOptions};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;

/// Shared resolver configuration for both the import index builder and the reference finder.
/// Kept in one place to prevent drift between the two resolution paths.
///
/// Accepts the workspace project list so it can build aliases that point bare package
/// imports (e.g. `@scope/contracts`) directly at their **source** roots instead of
/// letting the resolver follow `package.json` `exports`/`main` into `dist/`.
///
/// # Known limitation: `src/` heuristic
///
/// The alias-building logic below prefers `<project>/src/` when it exists. This works
/// well for the common Rush-style layout but will silently fall back to the project root
/// for packages that use a non-standard source directory (`lib/`, `source/`, etc.) or
/// keep sources at the project root without a subdirectory. A more principled approach
/// would be to resolve the package entry point normally, then check for a `source` field
/// in `package.json` (a convention used by several monorepo tools like Preconstruct).
/// This is left as a future improvement.
pub fn create_resolve_options(cwd: &Path, projects: &[Project]) -> ResolveOptions {
  let tsconfig_path = cwd.join("tsconfig.base.json");

  // Build aliases: @scope/pkg → <cwd>/<source_root>/src (or <source_root> if no src/ dir)
  // This ensures cross-package imports resolve to source files that Domino analyses,
  // rather than build output in dist/.
  //
  // Some workspace managers (e.g. Nx) already include /src in source_root, while others
  // (e.g. Rush) set source_root to the project folder.  When source_root points at a
  // project folder that contains a package.json, the resolver would follow exports/main
  // into dist/.  Pointing the alias at the src/ subdirectory bypasses package.json
  // entirely and lets main_files + extensions find index.ts directly.
  // The Nx project name (`my-lib`) and the package.json name (`@scope/my-lib`)
  // often differ. Alias both at the same source root so package-name imports,
  // including subpaths, resolve to source. oxc_resolver keeps the first alias
  // whose key matches, so each key is registered once and a package.json name
  // wins over another project's Nx name.
  let alias = workspace_aliases(cwd, projects);

  ResolveOptions {
    // TypeScript variants are listed before their JS counterparts (mirroring the
    // existing .ts-before-.js ordering) so that when both a TS and a JS file could
    // satisfy an extension-less specifier, the TypeScript source wins. The explicit
    // ESM/CJS variants (.mts/.cts/.mjs/.cjs) are common in strict-ESM monorepos and
    // dual-package (ESM+CJS) libraries; .d.mts/.d.cts fill the same declaration-file
    // slot as .d.ts.
    extensions: vec![
      ".ts".into(),
      ".tsx".into(),
      ".mts".into(),
      ".cts".into(),
      ".js".into(),
      ".jsx".into(),
      ".mjs".into(),
      ".cjs".into(),
      ".d.ts".into(),
      ".d.mts".into(),
      ".d.cts".into(),
    ],
    // Map .js/.jsx/.mjs/.cjs imports to their TypeScript equivalents.
    // Handles the common ESM pattern where .ts files import with .js extensions
    // (e.g., import { foo } from './bar.js' where the actual file is bar.ts), and
    // the analogous TypeScript "import with output extension" convention where a
    // .mts/.cts source is imported via its compiled .mjs/.cjs extension.
    // .mjs/.cjs are kept separate from .js (rather than folded into one shared
    // alias) so an ESM-explicit specifier can't silently resolve to a CJS-explicit
    // source or vice versa.
    extension_alias: vec![
      (
        ".js".into(),
        vec![".ts".into(), ".tsx".into(), ".js".into()],
      ),
      (".jsx".into(), vec![".tsx".into(), ".jsx".into()]),
      (".mjs".into(), vec![".mts".into(), ".mjs".into()]),
      (".cjs".into(), vec![".cts".into(), ".cjs".into()]),
    ],
    // Resolve bare package imports to source roots within the monorepo.
    alias,
    // condition_names and main_fields allow the resolver to follow bare specifiers
    // into package.json entry points for workspace-internal packages.
    // External node_modules specifiers are filtered out before reaching the resolver
    // via `is_workspace_specifier`, so only workspace packages incur this cost.
    condition_names: vec![
      "import".into(),
      "require".into(),
      "types".into(),
      "default".into(),
    ],
    main_fields: vec!["main".into(), "module".into(), "types".into()],
    main_files: vec!["index".into()],
    tsconfig: if tsconfig_path.exists() {
      Some(oxc_resolver::TsconfigDiscovery::Manual(
        oxc_resolver::TsconfigOptions {
          config_file: tsconfig_path,
          references: oxc_resolver::TsconfigReferences::Auto,
        },
      ))
    } else {
      None
    },
    ..Default::default()
  }
}

fn workspace_aliases(cwd: &Path, projects: &[Project]) -> Vec<(String, Vec<AliasValue>)> {
  let mut package_aliases = Vec::new();
  let mut project_aliases = Vec::with_capacity(projects.len());
  for project in projects {
    let target = alias_directory(cwd, project).to_string_lossy().into_owned();
    let package_name = distinct_package_name(cwd, project)
      .filter(|package_name| !package_aliases.iter().any(|(name, _)| name == package_name));
    if let Some(package_name) = package_name {
      package_aliases.push((package_name, target.clone()));
    }
    project_aliases.push((project.name.clone(), target));
  }

  let mut alias = Vec::with_capacity(package_aliases.len() + project_aliases.len());
  for (name, target) in package_aliases {
    alias.push((name, vec![AliasValue::Path(target)]));
  }
  for (name, target) in project_aliases {
    if alias.iter().any(|(key, _)| key == &name) {
      continue;
    }
    alias.push((name, vec![AliasValue::Path(target)]));
  }
  alias
}

fn alias_directory(cwd: &Path, project: &Project) -> PathBuf {
  let base = if project.source_root.is_absolute() {
    project.source_root.clone()
  } else {
    cwd.join(&project.source_root)
  };
  // Prefer <project>/src when it exists (Rush-style project folders).
  // If source_root already ends in src/ (Nx-style), or there is no src/ subdir,
  // use source_root as-is.
  if !base.ends_with("src") {
    let src_dir = base.join("src");
    if src_dir.is_dir() {
      return src_dir;
    }
  }
  base
}

fn absolute_under_cwd(cwd: &Path, path: &Path) -> PathBuf {
  if path.is_absolute() {
    path.to_path_buf()
  } else {
    cwd.join(path)
  }
}

fn package_name_at(path: &Path) -> Option<String> {
  let content = fs::read_to_string(path).ok()?;
  let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
  let name = parsed.get("name")?.as_str()?.trim();
  if name.is_empty() {
    None
  } else {
    Some(name.to_string())
  }
}

/// `package.json` `"name"` when it differs from the Nx project name.
///
/// Looks beside `project.json` (`project.root`). When `root` was stored as the
/// source directory (`.../src`), also checks that directory's parent — that is
/// where the package manifest sits.
fn distinct_package_name(cwd: &Path, project: &Project) -> Option<String> {
  let root = absolute_under_cwd(cwd, &project.root);
  if let Some(name) = package_name_at(&root.join("package.json")) {
    return (name != project.name).then_some(name);
  }

  let source = absolute_under_cwd(cwd, &project.source_root);
  if !source.ends_with("src") {
    return None;
  }
  let parent = source.parent()?;
  if parent == root {
    return None;
  }
  let name = package_name_at(&parent.join("package.json"))?;
  (name != project.name).then_some(name)
}

/// Package names imports actually use, for projects whose Nx name differs.
pub(crate) fn package_name_prefixes(cwd: &Path, projects: &[Project]) -> Vec<String> {
  let mut names = Vec::new();
  for project in projects {
    if let Some(name) = distinct_package_name(cwd, project) {
      if !names.iter().any(|existing| existing == &name) {
        names.push(name);
      }
    }
  }
  names
}

/// Returns `true` if the specifier is potentially workspace-internal and should be
/// passed to `oxc_resolver`. Relative and absolute specifiers always qualify.
/// Bare specifiers qualify if they match a known project name **or** a prefix in
/// `tsconfig_paths` (a tsconfig path key or a `package.json` name), including a
/// deep-import prefix like `@scope/pkg/sub/path`.
///
/// Both project names and those prefixes must be checked because they can differ:
/// an Nx project named `ui-widgets` may be imported as `@acme/shared-ui-widgets`
/// via a tsconfig path or as `@acme/ui-widgets` via `package.json`. Checking
/// only project names would misclassify such imports as external and silently
/// break cross-project reference tracking.
///
/// This avoids expensive filesystem I/O for external `node_modules` dependencies
/// (e.g. `react`, `lodash`) that the resolver would attempt to resolve via
/// `package.json` lookups before the `strip_prefix(cwd)` guard discards the result.
pub(crate) fn is_workspace_specifier(
  specifier: &str,
  projects: &[Project],
  tsconfig_paths: &[String],
) -> bool {
  if specifier.starts_with('.') || specifier.starts_with('/') {
    return true;
  }
  let matches_prefix = |prefix: &str| {
    specifier == prefix
      || (specifier.len() > prefix.len()
        && specifier.starts_with(prefix)
        && specifier.as_bytes()[prefix.len()] == b'/')
  };

  projects.iter().any(|p| matches_prefix(&p.name))
    || tsconfig_paths.iter().any(|p| matches_prefix(p))
}

#[derive(Deserialize)]
struct TsconfigJson {
  extends: Option<TsconfigExtends>,
  #[serde(rename = "compilerOptions")]
  compiler_options: Option<TsconfigCompilerOptions>,
}

/// TypeScript 5.0+ allows `extends` to be either a single string or an array of strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum TsconfigExtends {
  Single(String),
  Multiple(Vec<String>),
}

impl TsconfigExtends {
  fn into_vec(self) -> Vec<String> {
    match self {
      TsconfigExtends::Single(s) => vec![s],
      TsconfigExtends::Multiple(v) => v,
    }
  }
}

#[derive(Deserialize)]
struct TsconfigCompilerOptions {
  paths: Option<HashMap<String, Vec<String>>>,
}

/// Parse `tsconfig.base.json` (and its `extends` chain) and return the keys
/// from `compilerOptions.paths`.
///
/// These keys are the import specifiers that the TypeScript compiler (and oxc_resolver)
/// will resolve to workspace-internal paths. They often differ from the Nx project
/// names — e.g. a project named `ui-widgets` may be mapped as
/// `@acme/shared-ui-widgets` in tsconfig paths.
///
/// Wildcard suffixes (`/*`) are stripped so the returned strings can be used as
/// prefix-match candidates in `is_workspace_specifier`.
///
/// When the tsconfig uses `extends`, the chain is followed and paths are
/// inherited from ancestor configs — matching TypeScript semantics where the
/// leaf config's `paths` take precedence.
pub(crate) fn parse_tsconfig_path_prefixes(cwd: &Path) -> Vec<String> {
  let tsconfig_path = cwd.join("tsconfig.base.json");
  if !tsconfig_path.exists() {
    return vec![];
  }

  let paths = collect_tsconfig_paths(&tsconfig_path);

  paths
    .keys()
    .map(|key| key.strip_suffix("/*").unwrap_or(key).to_string())
    .collect()
}

fn read_tsconfig(path: &Path) -> Option<TsconfigJson> {
  let content = match std::fs::read_to_string(path) {
    Ok(c) => c,
    Err(e) => {
      warn!("Failed to read {}: {}", path.display(), e);
      return None;
    }
  };
  let stripped = StripComments::new(content.as_bytes());
  match serde_json::from_reader(stripped) {
    Ok(t) => Some(t),
    Err(e) => {
      warn!("Failed to parse {}: {}", path.display(), e);
      None
    }
  }
}

/// Walk the `extends` chain starting from `start_path`, collecting all
/// `compilerOptions.paths` entries. Child paths take precedence over parent
/// paths (matching TypeScript semantics).
///
/// Supports both single-string and array `extends` (TypeScript 5.0+).
/// Bare package specifiers (e.g. `@my-team/tsconfig-base`) are skipped because
/// resolving them would require full Node module resolution; only relative/absolute
/// paths are followed.
fn collect_tsconfig_paths(start_path: &Path) -> HashMap<String, Vec<String>> {
  let mut visited = std::collections::HashSet::new();
  let mut merged = HashMap::new();

  collect_tsconfig_paths_recursive(start_path, &mut visited, &mut merged, 0);

  merged
}

/// Safety net depth limit. The `visited` set is the primary cycle guard, but
/// `canonicalize()` can fail (returning the raw path), so two different textual
/// representations of the same file could bypass it. This cap prevents stack
/// overflow in that edge case.
const MAX_EXTENDS_DEPTH: usize = 64;

fn resolve_extends_specifier(parent_dir: &Path, specifier: &str) -> Option<PathBuf> {
  if !specifier.starts_with('.') && !specifier.starts_with('/') {
    warn!(
      "Skipping bare package extends specifier '{}' — only relative paths are supported",
      specifier
    );
    return None;
  }
  let mut path = parent_dir.join(specifier);
  if path.extension().is_none() {
    path.set_extension("json");
  }
  Some(path)
}

fn collect_tsconfig_paths_recursive(
  config_path: &Path,
  visited: &mut std::collections::HashSet<PathBuf>,
  merged: &mut HashMap<String, Vec<String>>,
  depth: usize,
) {
  if depth >= MAX_EXTENDS_DEPTH {
    warn!(
      "tsconfig extends chain exceeded {} levels at {} — possible cycle with non-canonical paths",
      MAX_EXTENDS_DEPTH,
      config_path.display()
    );
    return;
  }

  let canonical = config_path
    .canonicalize()
    .unwrap_or_else(|_| config_path.to_path_buf());
  if !visited.insert(canonical) {
    warn!("Circular extends detected at {}", config_path.display());
    return;
  }

  let tsconfig = match read_tsconfig(config_path) {
    Some(t) => t,
    None => return,
  };

  // Process parents first so child paths take precedence
  if let Some(extends) = tsconfig.extends {
    let parent_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    for specifier in extends.into_vec() {
      if let Some(parent_path) = resolve_extends_specifier(parent_dir, &specifier) {
        collect_tsconfig_paths_recursive(&parent_path, visited, merged, depth + 1);
      }
    }
  }

  // Child paths override parent paths
  if let Some(paths) = tsconfig.compiler_options.and_then(|co| co.paths) {
    merged.extend(paths);
  }
}

/// Extract the alias target path for a given package name from resolve options.
/// Returns `None` if the alias is not found.
#[cfg(test)]
fn alias_target(opts: &ResolveOptions, name: &str) -> Option<String> {
  opts.alias.iter().find_map(|(k, vs)| {
    if k == name {
      vs.first().and_then(|v| match v {
        AliasValue::Path(p) => Some(p.clone()),
        _ => None,
      })
    } else {
      None
    }
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::path::PathBuf;
  use tempfile::TempDir;

  #[test]
  fn test_alias_prefers_src_when_it_exists() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    // Create project with a src/ subdirectory
    let proj_dir = cwd.join("packages/my-lib");
    fs::create_dir_all(proj_dir.join("src")).unwrap();

    let projects = vec![Project {
      name: "@scope/my-lib".to_string(),
      root: PathBuf::from("packages/my-lib"),
      source_root: PathBuf::from("packages/my-lib"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    let opts = create_resolve_options(cwd, &projects);
    let target = alias_target(&opts, "@scope/my-lib").unwrap();

    assert!(
      target.ends_with("packages/my-lib/src"),
      "Expected alias to point at src/ subdir, got: {}",
      target
    );
  }

  #[test]
  fn test_alias_falls_back_when_no_src_dir() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    // Create project WITHOUT a src/ subdirectory (e.g. sources at root)
    let proj_dir = cwd.join("packages/my-lib");
    fs::create_dir_all(&proj_dir).unwrap();

    let projects = vec![Project {
      name: "@scope/my-lib".to_string(),
      root: PathBuf::from("packages/my-lib"),
      source_root: PathBuf::from("packages/my-lib"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    let opts = create_resolve_options(cwd, &projects);
    let target = alias_target(&opts, "@scope/my-lib").unwrap();

    assert!(
      target.ends_with("packages/my-lib"),
      "Expected alias to fall back to project root (no src/), got: {}",
      target
    );
    assert!(
      !target.ends_with("packages/my-lib/src"),
      "Should NOT point at non-existent src/ subdir, got: {}",
      target
    );
  }

  #[test]
  fn test_alias_skips_src_heuristic_when_source_root_already_ends_in_src() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    // Nx-style: source_root already includes /src
    let proj_dir = cwd.join("packages/my-lib/src");
    fs::create_dir_all(&proj_dir).unwrap();

    let projects = vec![Project {
      name: "@scope/my-lib".to_string(),
      root: PathBuf::from("packages/my-lib"),
      source_root: PathBuf::from("packages/my-lib/src"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    let opts = create_resolve_options(cwd, &projects);
    let target = alias_target(&opts, "@scope/my-lib").unwrap();

    assert!(
      target.ends_with("packages/my-lib/src"),
      "Expected alias to use source_root as-is (already ends in src), got: {}",
      target
    );
    // Must NOT double up to packages/my-lib/src/src
    assert!(
      !target.ends_with("src/src"),
      "Should not double-nest src/, got: {}",
      target
    );
  }

  #[test]
  fn test_no_panic_when_source_root_does_not_exist() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    // source_root points to a directory that doesn't exist at all
    let projects = vec![Project {
      name: "@scope/ghost".to_string(),
      root: PathBuf::from("packages/ghost"),
      source_root: PathBuf::from("packages/ghost"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    // Must not panic — is_dir() on non-existent path returns false
    let opts = create_resolve_options(cwd, &projects);
    let target = alias_target(&opts, "@scope/ghost").unwrap();

    assert!(
      target.ends_with("packages/ghost"),
      "Expected alias to fall back to (non-existent) project root, got: {}",
      target
    );
  }

  fn make_projects(names: &[&str]) -> Vec<Project> {
    names
      .iter()
      .map(|n| Project {
        name: n.to_string(),
        root: PathBuf::from("packages/placeholder"),
        source_root: PathBuf::from("packages/placeholder"),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      })
      .collect()
  }

  #[test]
  fn test_is_workspace_specifier_relative() {
    let projects = make_projects(&["@scope/lib"]);
    assert!(is_workspace_specifier("./utils", &projects, &[]));
    assert!(is_workspace_specifier("../shared/index", &projects, &[]));
  }

  #[test]
  fn test_is_workspace_specifier_absolute() {
    let projects = make_projects(&["@scope/lib"]);
    assert!(is_workspace_specifier("/absolute/path", &projects, &[]));
  }

  #[test]
  fn test_is_workspace_specifier_matching_project() {
    let projects = make_projects(&["@scope/lib", "shared-utils"]);
    assert!(is_workspace_specifier("@scope/lib", &projects, &[]));
    assert!(is_workspace_specifier(
      "@scope/lib/deep/path",
      &projects,
      &[]
    ));
    assert!(is_workspace_specifier("shared-utils", &projects, &[]));
    assert!(is_workspace_specifier(
      "shared-utils/helpers",
      &projects,
      &[]
    ));
  }

  #[test]
  fn test_is_workspace_specifier_external() {
    let projects = make_projects(&["@scope/lib", "shared-utils"]);
    assert!(!is_workspace_specifier("react", &projects, &[]));
    assert!(!is_workspace_specifier("lodash/fp", &projects, &[]));
    assert!(!is_workspace_specifier("@angular/core", &projects, &[]));
    assert!(!is_workspace_specifier("@scope/other-lib", &projects, &[]));
  }

  #[test]
  fn test_is_workspace_specifier_tsconfig_path_alias() {
    let projects = make_projects(&["ui-widgets"]);
    let tsconfig_paths = vec!["@acme/shared-ui-widgets".to_string()];

    assert!(
      !is_workspace_specifier("@acme/shared-ui-widgets", &projects, &[]),
      "should NOT match without tsconfig paths"
    );
    assert!(
      is_workspace_specifier("@acme/shared-ui-widgets", &projects, &tsconfig_paths),
      "should match with tsconfig path alias"
    );
    assert!(
      is_workspace_specifier(
        "@acme/shared-ui-widgets/deep/path",
        &projects,
        &tsconfig_paths
      ),
      "should match deep import under tsconfig path alias"
    );
    assert!(
      !is_workspace_specifier("react", &projects, &tsconfig_paths),
      "external packages should still be rejected"
    );
  }

  #[test]
  fn test_parse_tsconfig_path_prefixes() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    fs::write(
      cwd.join("tsconfig.base.json"),
      r#"{
        "compilerOptions": {
          "paths": {
            "@scope/my-lib": ["libs/my-lib/src/index.ts"],
            "@scope/other-lib/*": ["libs/other-lib/src/*"]
          }
        }
      }"#,
    )
    .unwrap();

    let prefixes = parse_tsconfig_path_prefixes(cwd);
    assert!(prefixes.contains(&"@scope/my-lib".to_string()));
    assert!(
      prefixes.contains(&"@scope/other-lib".to_string()),
      "wildcard suffix /* should be stripped"
    );
    assert_eq!(prefixes.len(), 2);
  }

  #[test]
  fn test_parse_tsconfig_path_prefixes_missing_file() {
    let tmp = TempDir::new().unwrap();
    let prefixes = parse_tsconfig_path_prefixes(tmp.path());
    assert!(prefixes.is_empty());
  }

  #[test]
  fn test_parse_tsconfig_path_prefixes_with_comments() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    fs::write(
      cwd.join("tsconfig.base.json"),
      r#"{
        // This is a line comment
        "compilerOptions": {
          /* block comment */
          "paths": {
            "@scope/my-lib": ["libs/my-lib/src/index.ts"],
            "@scope/other-lib/*": ["libs/other-lib/src/*"] // trailing comment
          }
        }
      }"#,
    )
    .unwrap();

    let prefixes = parse_tsconfig_path_prefixes(cwd);
    assert!(
      prefixes.contains(&"@scope/my-lib".to_string()),
      "should parse paths from JSONC with comments. Got: {:?}",
      prefixes
    );
    assert!(
      prefixes.contains(&"@scope/other-lib".to_string()),
      "should parse wildcard path from JSONC with comments. Got: {:?}",
      prefixes
    );
    assert_eq!(prefixes.len(), 2);
  }

  #[test]
  fn test_parse_tsconfig_path_prefixes_with_extends() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    fs::write(
      cwd.join("tsconfig.shared.json"),
      r#"{
        "compilerOptions": {
          "paths": {
            "@scope/shared-lib": ["libs/shared-lib/src/index.ts"],
            "@scope/overridden/*": ["libs/old-path/src/*"]
          }
        }
      }"#,
    )
    .unwrap();

    fs::write(
      cwd.join("tsconfig.base.json"),
      r#"{
        "extends": "./tsconfig.shared.json",
        "compilerOptions": {
          "paths": {
            "@scope/my-lib": ["libs/my-lib/src/index.ts"],
            "@scope/overridden/*": ["libs/new-path/src/*"]
          }
        }
      }"#,
    )
    .unwrap();

    let prefixes = parse_tsconfig_path_prefixes(cwd);
    assert!(
      prefixes.contains(&"@scope/shared-lib".to_string()),
      "should inherit paths from extended config. Got: {:?}",
      prefixes
    );
    assert!(
      prefixes.contains(&"@scope/my-lib".to_string()),
      "should include paths from leaf config. Got: {:?}",
      prefixes
    );
    assert!(
      prefixes.contains(&"@scope/overridden".to_string()),
      "child should override parent path keys. Got: {:?}",
      prefixes
    );
    assert_eq!(prefixes.len(), 3);
  }

  #[test]
  fn test_parse_tsconfig_path_prefixes_circular_extends() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    fs::write(
      cwd.join("tsconfig.a.json"),
      r#"{
        "extends": "./tsconfig.b.json",
        "compilerOptions": { "paths": { "@scope/a": ["a/src"] } }
      }"#,
    )
    .unwrap();

    fs::write(
      cwd.join("tsconfig.b.json"),
      r#"{
        "extends": "./tsconfig.a.json",
        "compilerOptions": { "paths": { "@scope/b": ["b/src"] } }
      }"#,
    )
    .unwrap();

    fs::write(
      cwd.join("tsconfig.base.json"),
      r#"{
        "extends": "./tsconfig.a.json",
        "compilerOptions": { "paths": { "@scope/root": ["root/src"] } }
      }"#,
    )
    .unwrap();

    let prefixes = parse_tsconfig_path_prefixes(cwd);
    assert!(
      prefixes.contains(&"@scope/root".to_string()),
      "should still return paths despite circular extends. Got: {:?}",
      prefixes
    );
  }

  #[test]
  fn test_parse_tsconfig_path_prefixes_array_extends() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    fs::write(
      cwd.join("tsconfig.paths-a.json"),
      r#"{
        "compilerOptions": {
          "paths": { "@scope/lib-a": ["libs/a/src/index.ts"] }
        }
      }"#,
    )
    .unwrap();

    fs::write(
      cwd.join("tsconfig.paths-b.json"),
      r#"{
        "compilerOptions": {
          "paths": { "@scope/lib-b": ["libs/b/src/index.ts"] }
        }
      }"#,
    )
    .unwrap();

    fs::write(
      cwd.join("tsconfig.base.json"),
      r#"{
        "extends": ["./tsconfig.paths-a.json", "./tsconfig.paths-b.json"],
        "compilerOptions": {
          "paths": { "@scope/root": ["libs/root/src/index.ts"] }
        }
      }"#,
    )
    .unwrap();

    let prefixes = parse_tsconfig_path_prefixes(cwd);
    assert!(
      prefixes.contains(&"@scope/lib-a".to_string()),
      "should inherit from first array extends entry. Got: {:?}",
      prefixes
    );
    assert!(
      prefixes.contains(&"@scope/lib-b".to_string()),
      "should inherit from second array extends entry. Got: {:?}",
      prefixes
    );
    assert!(
      prefixes.contains(&"@scope/root".to_string()),
      "should include paths from leaf config. Got: {:?}",
      prefixes
    );
    assert_eq!(prefixes.len(), 3);
  }

  #[test]
  fn test_parse_tsconfig_path_prefixes_bare_package_extends_skipped() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    fs::write(
      cwd.join("tsconfig.base.json"),
      r#"{
        "extends": "@my-team/tsconfig-base",
        "compilerOptions": {
          "paths": { "@scope/my-lib": ["libs/my-lib/src/index.ts"] }
        }
      }"#,
    )
    .unwrap();

    let prefixes = parse_tsconfig_path_prefixes(cwd);
    assert!(
      prefixes.contains(&"@scope/my-lib".to_string()),
      "should still return own paths when bare package extends is skipped. Got: {:?}",
      prefixes
    );
    assert_eq!(prefixes.len(), 1);
  }

  #[test]
  fn test_package_json_name_alias_resolves_subpath() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let src = cwd.join("libs/feature/src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("widget.ts"), "export const widget = 1;\n").unwrap();
    fs::write(
      cwd.join("libs/feature/package.json"),
      r#"{ "name": "@scope/my-lib" }"#,
    )
    .unwrap();

    let projects = vec![Project {
      name: "my-lib".to_string(),
      root: PathBuf::from("libs/feature"),
      source_root: PathBuf::from("libs/feature/src"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    let opts = create_resolve_options(cwd, &projects);
    let target = alias_target(&opts, "@scope/my-lib").unwrap();
    assert!(
      target.ends_with("libs/feature/src"),
      "package name should alias at the Nx source root, got: {target}"
    );

    let prefixes = package_name_prefixes(cwd, &projects);
    assert!(is_workspace_specifier(
      "@scope/my-lib/widget",
      &projects,
      &prefixes,
    ));
    assert!(
      !is_workspace_specifier("@scope/my-lib/widget", &projects, &[],),
      "without the package-name prefix the subpath is an external specifier"
    );

    let resolver = oxc_resolver::Resolver::new(opts);
    let resolved = resolver
      .resolve(cwd, "@scope/my-lib/widget")
      .expect("package subpath should resolve to source");
    assert!(
      resolved.path().ends_with("widget.ts"),
      "got {:?}",
      resolved.path()
    );
  }

  #[test]
  fn test_package_name_alias_wins_over_colliding_nx_name() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();

    let decoy = cwd.join("libs/decoy/src");
    fs::create_dir_all(&decoy).unwrap();
    fs::write(decoy.join("widget.ts"), "export const widget = 'decoy';\n").unwrap();
    fs::write(
      cwd.join("libs/decoy/package.json"),
      r#"{ "name": "@scope/my-lib" }"#,
    )
    .unwrap();

    let feature = cwd.join("libs/feature/src");
    fs::create_dir_all(&feature).unwrap();
    fs::write(
      feature.join("widget.ts"),
      "export const widget = 'feature';\n",
    )
    .unwrap();
    fs::write(
      cwd.join("libs/feature/package.json"),
      r#"{ "name": "@scope/my-lib" }"#,
    )
    .unwrap();

    let decoy_project = Project {
      name: "@scope/my-lib".to_string(),
      root: PathBuf::from("libs/decoy"),
      source_root: PathBuf::from("libs/decoy/src"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    };
    let feature_project = Project {
      name: "my-lib".to_string(),
      root: PathBuf::from("libs/feature"),
      source_root: PathBuf::from("libs/feature/src"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    };

    for projects in [
      vec![decoy_project.clone(), feature_project.clone()],
      vec![feature_project, decoy_project],
    ] {
      let opts = create_resolve_options(cwd, &projects);
      assert_eq!(
        opts
          .alias
          .iter()
          .filter(|(key, _)| key == "@scope/my-lib")
          .count(),
        1,
        "a colliding Nx name and package.json name share one alias"
      );
      let target = alias_target(&opts, "@scope/my-lib").unwrap();
      assert!(
        target.ends_with("libs/feature/src"),
        "package.json name should own the alias, got: {target}"
      );
      assert!(
        alias_target(&opts, "my-lib")
          .unwrap()
          .ends_with("libs/feature/src"),
        "the Nx name is still aliased when it does not collide"
      );

      let resolver = oxc_resolver::Resolver::new(opts);
      let resolved = resolver
        .resolve(cwd, "@scope/my-lib/widget")
        .expect("colliding specifier should resolve");
      let source = fs::read_to_string(resolved.path()).unwrap();
      assert!(
        source.contains("'feature'"),
        "resolved the decoy project: {source}"
      );
    }
  }

  #[test]
  fn test_package_name_read_from_parent_of_source_root() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    fs::create_dir_all(cwd.join("libs/feature/src")).unwrap();
    fs::write(
      cwd.join("libs/feature/package.json"),
      r#"{ "name": "@scope/my-lib" }"#,
    )
    .unwrap();

    let projects = vec![Project {
      name: "my-lib".to_string(),
      root: PathBuf::from("libs/feature/src"),
      source_root: PathBuf::from("libs/feature/src"),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    assert_eq!(
      package_name_prefixes(cwd, &projects),
      vec!["@scope/my-lib".to_string()]
    );
  }
}
