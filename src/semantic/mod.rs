pub mod analyzer;
pub mod assets;
pub mod reference_finder;
mod resolve_options;

use std::path::{Path, PathBuf};

pub use analyzer::WorkspaceAnalyzer;
pub use assets::AssetReferenceFinder;
pub use reference_finder::ReferenceFinder;
pub(crate) use resolve_options::create_resolve_options;

/// Shared fallback resolution for relative imports when oxc_resolver fails.
/// Handles .js/.jsx → .ts/.tsx remapping and standard extension probing.
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

  // 1. .js/.jsx → .ts/.tsx remapping (ESM convention)
  if let Some(stem) = specifier.strip_suffix(".js") {
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

  // 2. Standard extension probing + index file resolution
  let base = context.join(specifier);
  let base_str = base.to_string_lossy();
  for suffix in &[
    ".ts",
    ".tsx",
    ".js",
    ".jsx",
    "/index.ts",
    "/index.tsx",
    "/index.js",
    "/index.jsx",
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
