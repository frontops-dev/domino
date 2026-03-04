use crate::types::Project;
use oxc_resolver::{AliasValue, ResolveOptions};
use std::path::Path;

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
/// Bare specifiers qualify only if they match a known project name (possibly as a
/// deep-import prefix like `@scope/pkg/sub/path`).
///
/// This avoids expensive filesystem I/O for external `node_modules` dependencies
/// (e.g. `react`, `lodash`) that the resolver would attempt to resolve via
/// `package.json` lookups before the `strip_prefix(cwd)` guard discards the result.
pub(crate) fn is_workspace_specifier(specifier: &str, projects: &[Project]) -> bool {
  // Relative and absolute imports are always workspace-internal
  if specifier.starts_with('.') || specifier.starts_with('/') {
    return true;
  }
  // Bare specifier — check if it matches any project name (exact or prefix + '/')
  projects.iter().any(|p| {
    specifier == p.name || specifier.starts_with(&format!("{}/", p.name))
  })
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
    assert!(is_workspace_specifier("./utils", &projects));
    assert!(is_workspace_specifier("../shared/index", &projects));
  }

  #[test]
  fn test_is_workspace_specifier_absolute() {
    let projects = make_projects(&["@scope/lib"]);
    assert!(is_workspace_specifier("/absolute/path", &projects));
  }

  #[test]
  fn test_is_workspace_specifier_matching_project() {
    let projects = make_projects(&["@scope/lib", "shared-utils"]);
    assert!(is_workspace_specifier("@scope/lib", &projects));
    assert!(is_workspace_specifier("@scope/lib/deep/path", &projects));
    assert!(is_workspace_specifier("shared-utils", &projects));
    assert!(is_workspace_specifier("shared-utils/helpers", &projects));
  }

  #[test]
  fn test_is_workspace_specifier_external() {
    let projects = make_projects(&["@scope/lib", "shared-utils"]);
    assert!(!is_workspace_specifier("react", &projects));
    assert!(!is_workspace_specifier("lodash/fp", &projects));
    assert!(!is_workspace_specifier("@angular/core", &projects));
    assert!(!is_workspace_specifier("@scope/other-lib", &projects));
  }
}
