use crate::types::Project;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use tracing::warn;

/// The effective `compilerOptions.paths` of one tsconfig after its `extends` chain,
/// with the directory its targets resolve against.
///
/// Loaded by domino itself rather than by oxc_resolver: oxc_resolver fails the whole
/// config when an `extends` names a package it cannot find in `node_modules`, and CI
/// runs often install only the root package. A bare `extends` resolves through the
/// workspace project list first, so `@scope/ts-config/tsconfig.front.json` reaches
/// the project's sources without an install.
#[derive(Debug)]
pub(crate) struct TsconfigPaths {
  base_dir: PathBuf,
  patterns: PathPatterns,
}

/// `compilerOptions.paths` entries: a key with at most one `*`, and its target patterns.
type PathPatterns = Vec<(String, Vec<String>)>;

impl TsconfigPaths {
  /// `None` when the chain declares no `paths`.
  pub(crate) fn load(tsconfig: &Path, cwd: &Path, projects: &[Project]) -> Option<Self> {
    let effective = load_chain(tsconfig, cwd, projects, &mut Vec::new())?;
    let (patterns, paths_dir) = effective.paths?;
    Some(Self {
      base_dir: effective.base_url.unwrap_or(paths_dir),
      patterns,
    })
  }

  /// Absolute paths `specifier` maps to, in the order TypeScript tries them. Only the
  /// best pattern counts: an exact key, else the wildcard key with the longest prefix.
  pub(crate) fn candidates(&self, specifier: &str) -> Vec<PathBuf> {
    let exact = self
      .patterns
      .iter()
      .find(|(key, _)| !key.contains('*') && key == specifier)
      .map(|(_, targets)| (targets, ""));

    let wildcard = || {
      self
        .patterns
        .iter()
        .filter_map(|(key, targets)| {
          let (prefix, suffix) = key.split_once('*')?;
          let captured = specifier.strip_prefix(prefix)?.strip_suffix(suffix)?;
          Some((prefix.len(), targets, captured))
        })
        .max_by_key(|(prefix_len, _, _)| *prefix_len)
        .map(|(_, targets, captured)| (targets, captured))
    };

    exact
      .or_else(wildcard)
      .map(|(targets, captured)| {
        targets
          .iter()
          .map(|target| normalize(&self.base_dir.join(target.replacen('*', captured, 1))))
          .collect()
      })
      .unwrap_or_default()
  }
}

#[derive(Deserialize)]
struct TsconfigJson {
  extends: Option<Extends>,
  #[serde(rename = "compilerOptions")]
  compiler_options: Option<CompilerOptions>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Extends {
  Single(String),
  Multiple(Vec<String>),
}

#[derive(Deserialize)]
struct CompilerOptions {
  #[serde(rename = "baseUrl")]
  base_url: Option<String>,
  paths: Option<HashMap<String, Vec<String>>>,
}

#[derive(Default)]
struct EffectiveConfig {
  base_url: Option<PathBuf>,
  paths: Option<(PathPatterns, PathBuf)>,
}

/// Safety net for `extends` cycles the canonical-path check cannot see.
const MAX_EXTENDS_DEPTH: usize = 64;

fn load_chain(
  tsconfig: &Path,
  cwd: &Path,
  projects: &[Project],
  chain: &mut Vec<PathBuf>,
) -> Option<EffectiveConfig> {
  let canonical = tsconfig
    .canonicalize()
    .unwrap_or_else(|_| tsconfig.to_path_buf());
  if chain.contains(&canonical) || chain.len() >= MAX_EXTENDS_DEPTH {
    warn!("Circular tsconfig extends at {}", tsconfig.display());
    return None;
  }

  let json = read_tsconfig(tsconfig)?;
  let dir = tsconfig.parent()?.to_path_buf();

  chain.push(canonical);
  // TypeScript applies `extends` entries in order, each overriding the previous one,
  // then the config's own options override them all. `paths` is replaced, never merged.
  let mut effective = EffectiveConfig::default();
  let parents = match json.extends {
    Some(Extends::Single(specifier)) => vec![specifier],
    Some(Extends::Multiple(specifiers)) => specifiers,
    None => vec![],
  };
  for specifier in parents {
    let Some(parent) = resolve_extends(&dir, &specifier, cwd, projects) else {
      warn!(
        "Cannot resolve tsconfig extends '{}' from {}",
        specifier,
        tsconfig.display()
      );
      continue;
    };
    if let Some(inherited) = load_chain(&parent, cwd, projects, chain) {
      effective.base_url = inherited.base_url.or(effective.base_url);
      effective.paths = inherited.paths.or(effective.paths);
    }
  }
  chain.pop();

  if let Some(options) = json.compiler_options {
    if let Some(base_url) = options.base_url {
      effective.base_url = Some(dir.join(base_url));
    }
    if let Some(paths) = options.paths {
      effective.paths = Some((paths.into_iter().collect(), dir));
    }
  }
  Some(effective)
}

fn resolve_extends(
  dir: &Path,
  specifier: &str,
  cwd: &Path,
  projects: &[Project],
) -> Option<PathBuf> {
  if specifier.starts_with('.') || Path::new(specifier).is_absolute() {
    return existing_config(dir.join(specifier));
  }

  let workspace_project = projects
    .iter()
    .filter_map(|project| {
      let subpath = specifier.strip_prefix(project.name.as_str())?;
      let subpath = match subpath.strip_prefix('/') {
        Some(subpath) => subpath,
        None if subpath.is_empty() => "tsconfig.json",
        None => return None,
      };
      let root = if project.root.is_absolute() {
        project.root.clone()
      } else {
        cwd.join(&project.root)
      };
      existing_config(root.join(subpath))
    })
    .next();

  workspace_project.or_else(|| {
    dir
      .ancestors()
      .take_while(|ancestor| ancestor.starts_with(cwd))
      .find_map(|ancestor| {
        let package = ancestor.join("node_modules").join(specifier);
        if package.is_dir() {
          existing_config(package.join("tsconfig.json"))
        } else {
          existing_config(package)
        }
      })
  })
}

/// Drop `.` and fold `..` segments without touching the filesystem. Canonicalizing
/// would also resolve symlinks and move paths out from under the workspace root.
fn normalize(path: &Path) -> PathBuf {
  let mut normalized = PathBuf::new();
  for component in path.components() {
    match component {
      Component::CurDir => {}
      Component::ParentDir => {
        normalized.pop();
      }
      other => normalized.push(other),
    }
  }
  normalized
}

/// TypeScript accepts an `extends` target with or without its `.json` extension.
fn existing_config(path: PathBuf) -> Option<PathBuf> {
  if path.is_file() {
    return Some(path);
  }
  let mut with_extension = path.into_os_string();
  with_extension.push(".json");
  let with_extension = PathBuf::from(with_extension);
  with_extension.is_file().then_some(with_extension)
}

/// tsconfig files are JSONC: comments and trailing commas are both valid. The buffer
/// form of `json_strip_comments` removes both, the streaming reader only comments.
fn read_tsconfig(path: &Path) -> Option<TsconfigJson> {
  let mut content = std::fs::read_to_string(path)
    .map_err(|e| warn!("Failed to read {}: {}", path.display(), e))
    .ok()?;
  json_strip_comments::strip(&mut content)
    .map_err(|e| warn!("Failed to strip comments from {}: {}", path.display(), e))
    .ok()?;
  serde_json::from_str(&content)
    .map_err(|e| warn!("Failed to parse {}: {}", path.display(), e))
    .ok()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  fn project(name: &str, root: &str) -> Project {
    Project {
      name: name.to_string(),
      root: PathBuf::from(root),
      source_root: PathBuf::from(format!("{root}/src")),
      ts_config: None,
      implicit_dependencies: vec![],
      targets: vec![],
    }
  }

  #[test]
  fn test_bare_extends_resolves_through_workspace_project() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    fs::create_dir_all(cwd.join("tools/ts-config")).unwrap();
    fs::create_dir_all(cwd.join("apps/web")).unwrap();
    fs::write(
      cwd.join("tools/ts-config/tsconfig.front.json"),
      r#"{ "extends": "./tsconfig.base", "compilerOptions": { "jsx": "react-jsx" } }"#,
    )
    .unwrap();
    fs::write(
      cwd.join("tools/ts-config/tsconfig.base.json"),
      r#"{ "compilerOptions": { "paths": { "@shared/*": ["shared/*"] } } }"#,
    )
    .unwrap();
    fs::write(
      cwd.join("apps/web/tsconfig.json"),
      r#"{
  // no node_modules in this workspace
  "extends": "@scope/ts-config/tsconfig.front.json",
  "compilerOptions": { "paths": { "$pages/*": ["./src/pages/*"] } }
}"#,
    )
    .unwrap();

    let projects = vec![project("@scope/ts-config", "tools/ts-config")];
    let paths = TsconfigPaths::load(&cwd.join("apps/web/tsconfig.json"), cwd, &projects).unwrap();

    assert_eq!(
      paths.candidates("$pages/Fiche/index"),
      vec![cwd.join("apps/web/src/pages/Fiche/index")]
    );
    assert!(
      paths.candidates("@shared/x").is_empty(),
      "own `paths` replaces the inherited map"
    );
  }

  #[test]
  fn test_inherited_paths_resolve_against_declaring_config() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    fs::create_dir_all(cwd.join("apps/web")).unwrap();
    fs::write(
      cwd.join("tsconfig.shared.json"),
      r#"{ "compilerOptions": { "paths": { "@shared/*": ["libs/shared/*"] } } }"#,
    )
    .unwrap();
    fs::write(
      cwd.join("apps/web/tsconfig.json"),
      r#"{ "extends": "../../tsconfig.shared.json" }"#,
    )
    .unwrap();

    let paths = TsconfigPaths::load(&cwd.join("apps/web/tsconfig.json"), cwd, &[]).unwrap();

    assert_eq!(
      paths.candidates("@shared/button"),
      vec![cwd.join("libs/shared/button")]
    );
  }

  #[test]
  fn test_base_url_overrides_paths_directory() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    fs::write(
      cwd.join("tsconfig.json"),
      r#"{ "compilerOptions": { "baseUrl": "src", "paths": { "~/*": ["*"] } } }"#,
    )
    .unwrap();

    let paths = TsconfigPaths::load(&cwd.join("tsconfig.json"), cwd, &[]).unwrap();

    assert_eq!(paths.candidates("~/utils"), vec![cwd.join("src/utils")]);
  }

  #[test]
  fn test_candidates_prefer_exact_then_longest_prefix() {
    let paths = TsconfigPaths {
      base_dir: PathBuf::from("/ws"),
      patterns: vec![
        ("$i18n".to_string(), vec!["i18n/index".to_string()]),
        ("$i18n/*".to_string(), vec!["i18n/*".to_string()]),
        ("*".to_string(), vec!["types/*".to_string()]),
        (
          "$i18n/locales/*.json".to_string(),
          vec!["locales/*.json".to_string()],
        ),
      ],
    };

    assert_eq!(
      paths.candidates("$i18n"),
      vec![PathBuf::from("/ws/i18n/index")]
    );
    assert_eq!(
      paths.candidates("$i18n/fr"),
      vec![PathBuf::from("/ws/i18n/fr")]
    );
    assert_eq!(
      paths.candidates("$i18n/locales/fr.json"),
      vec![PathBuf::from("/ws/locales/fr.json")]
    );
    assert_eq!(
      paths.candidates("react"),
      vec![PathBuf::from("/ws/types/react")]
    );
  }

  #[test]
  fn test_trailing_commas_and_comments() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    fs::write(
      cwd.join("tsconfig.json"),
      r#"{
  // JSONC, as TypeScript accepts it
  "compilerOptions": {
    "paths": { "$pages/*": ["./src/pages/*",], },
  },
}"#,
    )
    .unwrap();

    let paths = TsconfigPaths::load(&cwd.join("tsconfig.json"), cwd, &[]).unwrap();

    assert_eq!(
      paths.candidates("$pages/Home"),
      vec![cwd.join("src/pages/Home")]
    );
  }

  /// TypeScript's `getPathsBasePath` is `options.baseUrl ?? options.pathsBasePath`: an
  /// effective `baseUrl` wins even when `paths` was declared by a parent config.
  #[test]
  fn test_child_base_url_applies_to_inherited_paths() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    fs::create_dir_all(cwd.join("apps/web")).unwrap();
    fs::write(
      cwd.join("tsconfig.shared.json"),
      r#"{ "compilerOptions": { "paths": { "@lib/*": ["lib/*"] } } }"#,
    )
    .unwrap();
    fs::write(
      cwd.join("apps/web/tsconfig.json"),
      r#"{ "extends": "../../tsconfig.shared.json", "compilerOptions": { "baseUrl": "." } }"#,
    )
    .unwrap();

    let paths = TsconfigPaths::load(&cwd.join("apps/web/tsconfig.json"), cwd, &[]).unwrap();

    assert_eq!(
      paths.candidates("@lib/button"),
      vec![cwd.join("apps/web/lib/button")]
    );
  }

  #[test]
  fn test_no_paths_in_chain() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    fs::write(
      cwd.join("tsconfig.json"),
      r#"{ "extends": "@missing/config" }"#,
    )
    .unwrap();

    assert!(TsconfigPaths::load(&cwd.join("tsconfig.json"), cwd, &[]).is_none());
  }
}
