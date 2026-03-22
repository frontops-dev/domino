use glob::Pattern;
use json_strip_comments::StripComments;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::warn;

const MAX_EXTENDS_DEPTH: usize = 64;

#[derive(Deserialize)]
struct TsconfigFile {
  extends: Option<TsconfigExtends>,
  #[serde(default)]
  exclude: Option<Vec<String>>,
}

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

/// Compiled exclude patterns from a project's tsconfig, used to filter
/// files that shouldn't count toward project ownership (e.g. stories, specs).
#[derive(Debug)]
pub struct TsconfigExcludes {
  patterns: Vec<Pattern>,
  /// Directory containing the tsconfig — patterns are relative to this.
  base_dir: PathBuf,
}

impl TsconfigExcludes {
  /// Parse a tsconfig file and extract its `exclude` patterns, following the
  /// `extends` chain. Returns `None` if the tsconfig doesn't exist, can't be
  /// parsed, or has no exclude patterns.
  pub fn parse(tsconfig_path: &Path, cwd: &Path) -> Option<Self> {
    let base_dir = tsconfig_path.parent()?.to_path_buf();

    let excludes = collect_excludes(tsconfig_path);
    if excludes.is_empty() {
      return None;
    }

    let patterns: Vec<Pattern> = excludes
      .iter()
      .filter_map(|pat| match Pattern::new(pat) {
        Ok(p) => Some(p),
        Err(e) => {
          warn!(
            "Invalid exclude pattern '{}' in {}: {}",
            pat,
            tsconfig_path.display(),
            e
          );
          None
        }
      })
      .collect();

    if patterns.is_empty() {
      return None;
    }

    let rel_base = base_dir
      .strip_prefix(cwd)
      .unwrap_or(&base_dir)
      .to_path_buf();

    Some(Self {
      patterns,
      base_dir: rel_base,
    })
  }

  pub fn pattern_count(&self) -> usize {
    self.patterns.len()
  }

  /// Check if a workspace-relative file path is excluded by this tsconfig.
  pub fn is_excluded(&self, file_rel_path: &Path) -> bool {
    let relative = match file_rel_path.strip_prefix(&self.base_dir) {
      Ok(r) => r,
      Err(_) => return false,
    };

    let rel_str = match relative.to_str() {
      Some(s) => s,
      None => return false,
    };

    self
      .patterns
      .iter()
      .any(|p| p.matches_with(rel_str, glob_match_options()))
  }
}

fn glob_match_options() -> glob::MatchOptions {
  glob::MatchOptions {
    case_sensitive: true,
    require_literal_separator: false,
    require_literal_leading_dot: false,
  }
}

fn read_tsconfig_file(path: &Path) -> Option<TsconfigFile> {
  let content = match std::fs::read_to_string(path) {
    Ok(c) => c,
    Err(_) => return None,
  };
  let stripped = StripComments::new(content.as_bytes());
  match serde_json::from_reader(stripped) {
    Ok(t) => Some(t),
    Err(e) => {
      warn!("Failed to parse tsconfig at {}: {}", path.display(), e);
      None
    }
  }
}

fn resolve_extends(parent_dir: &Path, specifier: &str) -> Option<PathBuf> {
  if !specifier.starts_with('.') && !specifier.starts_with('/') {
    return None;
  }
  let mut path = parent_dir.join(specifier);
  if path.extension().is_none() {
    path.set_extension("json");
  }
  Some(path)
}

/// Walk the `extends` chain and collect all `exclude` patterns.
/// Child excludes fully replace parent excludes (matching TypeScript semantics).
fn collect_excludes(start_path: &Path) -> Vec<String> {
  let mut visited = HashSet::new();
  collect_excludes_recursive(start_path, &mut visited, 0)
}

fn collect_excludes_recursive(
  config_path: &Path,
  visited: &mut HashSet<PathBuf>,
  depth: usize,
) -> Vec<String> {
  if depth >= MAX_EXTENDS_DEPTH {
    return vec![];
  }

  let canonical = config_path
    .canonicalize()
    .unwrap_or_else(|_| config_path.to_path_buf());
  if !visited.insert(canonical) {
    return vec![];
  }

  let tsconfig = match read_tsconfig_file(config_path) {
    Some(t) => t,
    None => return vec![],
  };

  // If this config has its own excludes, use them (child overrides parent entirely).
  if let Some(excludes) = tsconfig.exclude {
    return excludes;
  }

  // No excludes here — inherit from parent(s).
  if let Some(extends) = tsconfig.extends {
    let parent_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    for specifier in extends.into_vec() {
      if let Some(parent_path) = resolve_extends(parent_dir, &specifier) {
        let inherited = collect_excludes_recursive(&parent_path, visited, depth + 1);
        if !inherited.is_empty() {
          return inherited;
        }
      }
    }
  }

  vec![]
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  #[test]
  fn test_basic_exclude_matching() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let lib_dir = cwd.join("libs/my-lib");
    fs::create_dir_all(&lib_dir).unwrap();

    fs::write(
      lib_dir.join("tsconfig.lib.json"),
      r#"{
        "exclude": [
          "**/*.spec.ts",
          "**/*.stories.tsx",
          "jest.config.ts"
        ]
      }"#,
    )
    .unwrap();

    let excludes =
      TsconfigExcludes::parse(&lib_dir.join("tsconfig.lib.json"), cwd).expect("should parse");

    assert!(excludes.is_excluded(Path::new("libs/my-lib/src/utils.spec.ts")));
    assert!(excludes.is_excluded(Path::new("libs/my-lib/src/components/Grid.stories.tsx")));
    assert!(excludes.is_excluded(Path::new("libs/my-lib/jest.config.ts")));

    assert!(!excludes.is_excluded(Path::new("libs/my-lib/src/utils.ts")));
    assert!(!excludes.is_excluded(Path::new("libs/my-lib/src/components/Grid.tsx")));
  }

  #[test]
  fn test_no_exclude_returns_none() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let lib_dir = cwd.join("libs/my-lib");
    fs::create_dir_all(&lib_dir).unwrap();

    fs::write(
      lib_dir.join("tsconfig.json"),
      r#"{ "compilerOptions": { "strict": true } }"#,
    )
    .unwrap();

    let result = TsconfigExcludes::parse(&lib_dir.join("tsconfig.json"), cwd);
    assert!(result.is_none());
  }

  #[test]
  fn test_missing_tsconfig_returns_none() {
    let tmp = TempDir::new().unwrap();
    let result = TsconfigExcludes::parse(&tmp.path().join("nonexistent.json"), tmp.path());
    assert!(result.is_none());
  }

  #[test]
  fn test_tsconfig_with_comments() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let lib_dir = cwd.join("libs/my-lib");
    fs::create_dir_all(&lib_dir).unwrap();

    fs::write(
      lib_dir.join("tsconfig.lib.json"),
      r#"{
        // Build config for this library
        "exclude": [
          "**/*.spec.ts", // test files
          "**/*.stories.tsx" /* storybook files */
        ]
      }"#,
    )
    .unwrap();

    let excludes =
      TsconfigExcludes::parse(&lib_dir.join("tsconfig.lib.json"), cwd).expect("should parse JSONC");
    assert!(excludes.is_excluded(Path::new("libs/my-lib/src/index.spec.ts")));
    assert!(excludes.is_excluded(Path::new("libs/my-lib/src/Button.stories.tsx")));
  }

  #[test]
  fn test_excludes_inherited_from_parent() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let lib_dir = cwd.join("libs/my-lib");
    fs::create_dir_all(&lib_dir).unwrap();

    fs::write(
      lib_dir.join("tsconfig.base.json"),
      r#"{ "exclude": ["**/*.spec.ts", "**/*.stories.tsx"] }"#,
    )
    .unwrap();

    fs::write(
      lib_dir.join("tsconfig.lib.json"),
      r#"{ "extends": "./tsconfig.base.json" }"#,
    )
    .unwrap();

    let excludes =
      TsconfigExcludes::parse(&lib_dir.join("tsconfig.lib.json"), cwd).expect("should inherit");
    assert!(excludes.is_excluded(Path::new("libs/my-lib/src/foo.spec.ts")));
    assert!(excludes.is_excluded(Path::new("libs/my-lib/src/Bar.stories.tsx")));
  }

  #[test]
  fn test_child_excludes_override_parent() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let lib_dir = cwd.join("libs/my-lib");
    fs::create_dir_all(&lib_dir).unwrap();

    fs::write(
      lib_dir.join("tsconfig.base.json"),
      r#"{ "exclude": ["**/*.spec.ts", "**/*.stories.tsx"] }"#,
    )
    .unwrap();

    fs::write(
      lib_dir.join("tsconfig.lib.json"),
      r#"{
        "extends": "./tsconfig.base.json",
        "exclude": ["**/*.spec.ts"]
      }"#,
    )
    .unwrap();

    let excludes =
      TsconfigExcludes::parse(&lib_dir.join("tsconfig.lib.json"), cwd).expect("should parse");
    assert!(excludes.is_excluded(Path::new("libs/my-lib/src/foo.spec.ts")));
    assert!(
      !excludes.is_excluded(Path::new("libs/my-lib/src/Bar.stories.tsx")),
      "child exclude should fully replace parent, not merge"
    );
  }

  #[test]
  fn test_file_outside_base_dir_not_excluded() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let lib_dir = cwd.join("libs/my-lib");
    fs::create_dir_all(&lib_dir).unwrap();

    fs::write(
      lib_dir.join("tsconfig.lib.json"),
      r#"{ "exclude": ["**/*.spec.ts"] }"#,
    )
    .unwrap();

    let excludes =
      TsconfigExcludes::parse(&lib_dir.join("tsconfig.lib.json"), cwd).expect("should parse");

    assert!(
      !excludes.is_excluded(Path::new("libs/other-lib/src/index.spec.ts")),
      "files outside the tsconfig's directory should never be excluded"
    );
  }

  #[test]
  fn test_circular_extends() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let lib_dir = cwd.join("libs/my-lib");
    fs::create_dir_all(&lib_dir).unwrap();

    fs::write(
      lib_dir.join("a.json"),
      r#"{ "extends": "./b.json", "exclude": ["**/*.spec.ts"] }"#,
    )
    .unwrap();
    fs::write(lib_dir.join("b.json"), r#"{ "extends": "./a.json" }"#).unwrap();

    let excludes =
      TsconfigExcludes::parse(&lib_dir.join("a.json"), cwd).expect("should handle circular");
    assert!(excludes.is_excluded(Path::new("libs/my-lib/foo.spec.ts")));
  }
}
