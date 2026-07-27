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
/// Handles .js/.jsx/.mjs/.cjs → TypeScript-equivalent remapping and standard
/// extension probing. Mirrors the `extensions` / `extension_alias` config in
/// `create_resolve_options` — kept in sync with it deliberately.
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

  // 1. .js/.jsx/.mjs/.cjs → TypeScript-equivalent remapping (ESM convention).
  // .mjs and .cjs are handled as separate branches (rather than merged into the
  // .js branch) so an ESM-explicit specifier can't silently resolve to a
  // CJS-explicit source or vice versa.
  if let Some(stem) = specifier.strip_suffix(".mjs") {
    let stem_path = context.join(stem);
    let stem_str = stem_path.to_string_lossy();
    for ext in &[".mts", ".mjs"] {
      let candidate = PathBuf::from(format!("{}{}", stem_str, ext));
      if let Some(p) = try_candidate(&candidate) {
        return Some(p);
      }
    }
  } else if let Some(stem) = specifier.strip_suffix(".cjs") {
    let stem_path = context.join(stem);
    let stem_str = stem_path.to_string_lossy();
    for ext in &[".cts", ".cjs"] {
      let candidate = PathBuf::from(format!("{}{}", stem_str, ext));
      if let Some(p) = try_candidate(&candidate) {
        return Some(p);
      }
    }
  } else if let Some(stem) = specifier.strip_suffix(".js") {
    let stem_path = context.join(stem);
    let stem_str = stem_path.to_string_lossy();
    for ext in &[".ts", ".tsx", ".js"] {
      let candidate = PathBuf::from(format!("{}{}", stem_str, ext));
      if let Some(p) = try_candidate(&candidate) {
        return Some(p);
      }
    }
  } else if let Some(stem) = specifier.strip_suffix(".jsx") {
    let stem_path = context.join(stem);
    let stem_str = stem_path.to_string_lossy();
    for ext in &[".tsx", ".jsx"] {
      let candidate = PathBuf::from(format!("{}{}", stem_str, ext));
      if let Some(p) = try_candidate(&candidate) {
        return Some(p);
      }
    }
  }

  // 2. Standard extension probing + index file resolution.
  // TypeScript variants precede their JS counterparts, matching the ordering
  // in `create_resolve_options`.
  let base = context.join(specifier);
  let base_str = base.to_string_lossy();
  for suffix in &[
    ".ts",
    ".tsx",
    ".mts",
    ".cts",
    ".js",
    ".jsx",
    ".mjs",
    ".cjs",
    "/index.ts",
    "/index.tsx",
    "/index.mts",
    "/index.cts",
    "/index.js",
    "/index.jsx",
    "/index.mjs",
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
