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
    // NOTE: condition_names and main_fields allow the resolver to follow bare specifiers
    // into package.json entry points, which is necessary for correct monorepo resolution.
    // However, this also means the resolver will attempt resolution for external
    // node_modules dependencies. The `strip_prefix(cwd)` guard in `resolve_import`
    // correctly drops paths outside the workspace root, but the resolution work still
    // happens. In a large monorepo with hundreds of third-party imports per file, this
    // could meaningfully impact performance. Worth benchmarking on a realistic workspace.
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
