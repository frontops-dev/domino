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
  use crate::types::{LockfileStrategy, Project, TrueAffectedConfig};
  use std::fs;
  use std::process::Command;
  use std::sync::Arc;
  use tempfile::TempDir;

  fn git(root: &std::path::Path, args: &[&str]) -> String {
    let o = Command::new("git").args(args).current_dir(root).output().unwrap();
    String::from_utf8_lossy(&o.stdout).to_string()
  }

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
      "import { Widget } from '../../lib/src/Widget.jsx';\n\nexport function main() {\n  return Widget;\n}\n",
    )
    .unwrap();
    fs::write(root.join("lib/package.json"), r#"{"name":"@test/lib","version":"0.0.0"}"#).unwrap();
    fs::write(root.join("app/package.json"), r#"{"name":"@test/app","version":"0.0.0"}"#).unwrap();

    git(&root, &["init"]);
    git(&root, &["config", "user.email", "t@t.com"]);
    git(&root, &["config", "user.name", "T"]);
    git(&root, &["branch", "-M", "main"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    git(&root, &["checkout", "-b", "feature"]);
    fs::write(
      lib_src.join("Widget.tsx"),
      "export const Widget = () => <div>modified</div>;\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "modify Widget"]);

    let mut out = String::new();
    out.push_str(&format!("\nROOT={root:?}\n"));
    out.push_str(&format!("RAW_DIFF:\n{}\n", git(&root, &["diff", "main...HEAD"])));
    out.push_str(&format!(
      "CHANGED_FILES: {:?}\n",
      crate::git::get_changed_files(&root, "main", None)
    ));

    let mk = |n: &str| Project {
      name: n.to_string(),
      root: n.into(),
      source_root: n.into(),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    };
    let config = TrueAffectedConfig {
      cwd: root.clone(),
      base: "main".to_string(),
      head: None,
      root_ts_config: None,
      projects: vec![mk("lib"), mk("app")],
      include: vec![],
      ignored_paths: vec![],
      lockfile_strategy: LockfileStrategy::None,
    };
    let profiler = Arc::new(Profiler::new(false));
    let report = crate::core::find_affected_with_report(config, profiler).unwrap();
    out.push_str(&format!("AFFECTED: {:?}\n", report.affected_projects));
    out.push_str(&format!("REPORT: {:#?}\n", report.report));

    panic!("DIAGNOSTIC OUTPUT (intentional failure):{out}");
  }
}
