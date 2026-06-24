pub mod analyzer;
pub mod assets;
pub mod reference_finder;
mod resolve_options;

use std::path::{Path, PathBuf};

pub use analyzer::WorkspaceAnalyzer;
pub use assets::AssetReferenceFinder;
pub use reference_finder::ReferenceFinder;
pub(crate) use resolve_options::create_resolve_options;
pub(crate) use resolve_options::is_workspace_specifier;
pub(crate) use resolve_options::parse_tsconfig_path_prefixes;

/// Shared fallback resolution for relative imports when oxc_resolver fails.
/// Handles .js/.jsx/.mjs/.cjs → .ts/.tsx/.mts/.cts remapping and standard
/// extension probing.
pub(crate) fn simple_resolve_relative(
  cwd: &Path,
  context: &Path,
  specifier: &str,
) -> Option<PathBuf> {
  if !specifier.starts_with('.') {
    return None;
  }

  let try_candidate = |candidate: &Path| -> Option<PathBuf> {
    if cwd.join(candidate).exists() {
      candidate.strip_prefix(cwd).ok().map(|p| p.to_path_buf())
    } else {
      None
    }
  };

  // 1. .js/.jsx/.mjs/.cjs → .ts/.tsx/.mts/.cts remapping (ESM convention).
  // TypeScript ESM emits `.mjs` specifiers for `.mts` sources (and `.cjs` for
  // `.cts`), so a barrel doing `export * from "./foo.mjs"` must resolve to
  // `foo.mts`.
  let ext_remap: &[(&str, &[&str])] = &[
    (".js", &[".ts", ".tsx", ".js"]),
    (".jsx", &[".tsx", ".jsx"]),
    (".mjs", &[".mts", ".mjs"]),
    (".cjs", &[".cts", ".cjs"]),
  ];
  for (suffix, candidates) in ext_remap {
    if let Some(stem) = specifier.strip_suffix(suffix) {
      let stem_path = context.join(stem);
      let stem_str = stem_path.to_string_lossy();
      for ext in *candidates {
        let candidate = PathBuf::from(format!("{}{}", stem_str, ext));
        if let Some(p) = try_candidate(&candidate) {
          return Some(p);
        }
      }
      break;
    }
  }

  // 2. Standard extension probing + index file resolution
  let base = context.join(specifier);
  let base_str = base.to_string_lossy();
  for suffix in &[
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    ".mts",
    ".mjs",
    ".cts",
    ".cjs",
    "/index.ts",
    "/index.tsx",
    "/index.js",
    "/index.jsx",
    "/index.mts",
    "/index.mjs",
    "/index.cts",
    "/index.cjs",
  ] {
    let candidate = if let Some(stripped) = suffix.strip_prefix('/') {
      base.join(stripped)
    } else {
      PathBuf::from(format!("{}{}", base_str, suffix))
    };
    if let Some(p) = try_candidate(&candidate) {
      return Some(p);
    }
  }

  None
}
