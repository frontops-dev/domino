use crate::types::Project;
use json_strip_comments::StripComments;
use oxc_resolver::{AliasValue, ResolveOptions};
use serde::Deserialize;
use std::collections::HashMap;
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
  let alias = projects
    .iter()
    .map(|p| {
      let base = if p.source_root.is_absolute() {
        p.source_root.clone()
      } else {
        cwd.join(&p.source_root)
      };
      // Prefer <project>/src when it exists (Rush-style project folders).
      // If source_root already ends in src/ (Nx-style), or there is no src/ subdir,
      // use source_root as-is.
      let target = if !base.ends_with("src") {
        let src_dir = base.join("src");
        if src_dir.is_dir() {
          src_dir
        } else {
          base
        }
      } else {
        base
      };
      (
        p.name.clone(),
        vec![AliasValue::Path(target.to_string_lossy().into_owned())],
      )
    })
    .collect::<Vec<_>>();

  ResolveOptions {
    extensions: vec![
      ".ts".into(),
      ".tsx".into(),
      ".js".into(),
      ".jsx".into(),
      ".d.ts".into(),
    ],
    // Map .js/.jsx imports to their TypeScript equivalents.
    // Handles the common ESM pattern where .ts files import with .js extensions
    // (e.g., import { foo } from './bar.js' where the actual file is bar.ts).
    extension_alias: vec![
      (
        ".js".into(),
        vec![".ts".into(), ".tsx".into(), ".js".into()],
      ),
      (".jsx".into(), vec![".tsx".into(), ".jsx".into()]),
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

/// Returns `true` if the specifier is potentially workspace-internal and should be
/// passed to `oxc_resolver`. Relative and absolute specifiers always qualify.
/// Bare specifiers qualify if they match a known project name **or** a tsconfig
/// path alias (possibly as a deep-import prefix like `@scope/pkg/sub/path`).
///
/// Both project names and tsconfig paths must be checked because they can differ:
/// an Nx project named `ui-widgets` may be imported as `@acme/shared-ui-widgets`
/// via a tsconfig path alias. Checking only project names would misclassify
/// such imports as external and silently break cross-project reference tracking.
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
  let matches_prefix =
    |prefix: &str| specifier == prefix || specifier.starts_with(&format!("{}/", prefix));

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
/// When the tsconfig uses `extends`, the chain is followed (up to 10 levels)
/// and paths are inherited from ancestor configs — matching TypeScript semantics
/// where the leaf config's `paths` take precedence.
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
}
