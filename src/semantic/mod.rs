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

#[cfg(test)]
mod diag_tests {
  use super::*;
  use crate::profiler::Profiler;
  use crate::types::Project;
  use oxc_resolver::Resolver;
  use std::fs;
  use std::sync::Arc;
  use tempfile::TempDir;

  #[test]
  fn diag_jsx_resolution() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let lib_src = root.join("lib/src");
    let app_src = root.join("app/src");
    fs::create_dir_all(&lib_src).unwrap();
    fs::create_dir_all(&app_src).unwrap();
    fs::write(lib_src.join("Widget.tsx"), "export const Widget = () => null;\n").unwrap();
    fs::write(
      app_src.join("index.ts"),
      "import { Widget } from '../../lib/src/Widget.jsx';\nexport function main() { return Widget; }\n",
    )
    .unwrap();
    fs::write(root.join("lib/package.json"), r#"{"name":"@test/lib","version":"0.0.0"}"#).unwrap();
    fs::write(root.join("app/package.json"), r#"{"name":"@test/app","version":"0.0.0"}"#).unwrap();

    let mk = |n: &str| Project {
      name: n.to_string(),
      root: n.into(),
      source_root: n.into(),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    };
    let projects = vec![mk("lib"), mk("app")];

    let mut out = String::new();
    out.push_str(&format!("\nROOT={root:?}\n"));

    let resolver = Resolver::new(create_resolve_options(&root, &projects));
    let ctx = root.join("app/src");
    let spec = "../../lib/src/Widget.jsx";
    out.push_str(&format!(
      "RESOLVER: {:?}\n",
      resolver.resolve(&ctx, spec).map(|r| r.full_path())
    ));
    out.push_str(&format!(
      "FALLBACK: {:?}\n",
      simple_resolve_relative(&root, &ctx, spec)
    ));

    let profiler = Arc::new(Profiler::new(false));
    let analyzer = WorkspaceAnalyzer::new(projects, &root, profiler).unwrap();
    let mut parsed: Vec<_> = analyzer.files.keys().collect();
    parsed.sort();
    out.push_str(&format!("PARSED: {parsed:?}\n"));
    out.push_str(&format!("IMPORTS: {:?}\n", analyzer.imports));
    let mut idx: Vec<_> = analyzer.import_index.keys().collect();
    idx.sort();
    out.push_str(&format!("INDEX_KEYS: {idx:?}\n"));

    panic!("DIAGNOSTIC OUTPUT (intentional failure):{out}");
  }
}
