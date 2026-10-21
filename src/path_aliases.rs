//! Narrowing a change to the workspace-root `tsconfig.base.json` to the path
//! aliases it added, removed or re-targeted.
//!
//! When only `compilerOptions.paths` changed, the code that can behave
//! differently is the code whose import specifiers those aliases match: an
//! added alias captures imports of its name, a removed or re-targeted one sends
//! them somewhere else. Any other change (another compiler option, `baseUrl`,
//! `extends`) can affect every project, so it is not narrowed.
//!
//! Only head sources need searching. An importer that did not change reads the
//! same at base and head, and one that did change is in the diff anyway.

use crate::git;
use crate::semantic::ROOT_TSCONFIG_FILE;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tracing::debug;

/// A `compilerOptions.paths` key, compiled for matching import specifiers.
#[derive(Debug)]
pub struct AliasPattern {
  /// The key as written in `paths`
  pub alias: String,
  /// The whole key, or for a `prefix/*` key everything up to and including `/`
  prefix: String,
  wildcard: bool,
  /// A quoted string literal holding a matching specifier
  quoted: Regex,
}

impl AliasPattern {
  /// `None` for keys this matching does not model: an empty key, or a `*`
  /// anywhere except a trailing `/*`.
  pub fn new(alias: &str) -> Option<Self> {
    let (prefix, wildcard) = match alias.strip_suffix("/*") {
      Some(stem) => (format!("{stem}/"), true),
      None => (alias.to_string(), false),
    };
    if prefix.is_empty() || prefix.contains('*') {
      return None;
    }
    // Backticks too: a template-literal `import()` or `require()` only adds
    // matches, so including them can over-select but never miss an importer.
    let body = if wildcard {
      format!(r"{}[^'`\x22\n]+", regex::escape(&prefix))
    } else {
      regex::escape(&prefix)
    };
    let quoted = Regex::new(&format!(r"['`\x22]{body}['`\x22]")).ok()?;
    Some(Self {
      alias: alias.to_string(),
      prefix,
      wildcard,
      quoted,
    })
  }

  /// Whether an import specifier resolves through this alias.
  pub fn matches(&self, specifier: &str) -> bool {
    if self.wildcard {
      specifier.len() > self.prefix.len() && specifier.starts_with(&self.prefix)
    } else {
      specifier == self.prefix
    }
  }

  /// Whether `source` has a string literal holding a matching specifier. This
  /// finds the forms the analyzer records no import for: side-effect imports,
  /// `require()` and non-literal `import()` arguments.
  pub fn mentioned_in(&self, source: &str) -> bool {
    // The substring check is a cheap filter: most files never mention the alias.
    source.contains(self.prefix.as_str()) && self.quoted.is_match(source)
  }
}

/// The aliases a change from `base` to `head` added, removed or re-targeted,
/// or `None` when the change cannot be narrowed: either side does not parse,
/// something outside `compilerOptions.paths` differs, or a changed key is a
/// pattern [`AliasPattern`] does not model.
pub fn changed_aliases(base: &str, head: &str) -> Option<Vec<AliasPattern>> {
  let mut base = parse_jsonc(base)?;
  let mut head = parse_jsonc(head)?;
  let base_paths = take_paths(&mut base)?;
  let head_paths = take_paths(&mut head)?;
  if base != head {
    return None;
  }
  // A child's `paths` replaces the one it `extends` wholesale, so declaring or
  // dropping it swaps inherited aliases this diff cannot see.
  if base.get("extends").is_some() && base_paths.is_some() != head_paths.is_some() {
    return None;
  }
  let base_paths = base_paths.unwrap_or_default();
  let head_paths = head_paths.unwrap_or_default();

  let changed: BTreeSet<&String> = base_paths
    .keys()
    .chain(head_paths.keys())
    .filter(|key| base_paths.get(*key) != head_paths.get(*key))
    .collect();
  changed
    .into_iter()
    .map(|key| AliasPattern::new(key))
    .collect()
}

/// [`changed_aliases`] for the root tsconfig at `merge_base` and at `head` (the
/// working tree when `None`). `None` also when the file is missing on either
/// side.
pub fn changed_aliases_between(
  cwd: &Path,
  merge_base: &str,
  head: Option<&str>,
) -> Option<Vec<AliasPattern>> {
  let path = Path::new(ROOT_TSCONFIG_FILE);
  let read_revision = |revision: &str| match git::get_file_at_revision(cwd, revision, path) {
    Ok(Some(text)) => Some(text),
    Ok(None) => {
      debug!("{ROOT_TSCONFIG_FILE} is missing at {revision}; not narrowing");
      None
    }
    Err(e) => {
      debug!("Could not read {ROOT_TSCONFIG_FILE} at {revision} ({e}); not narrowing");
      None
    }
  };
  let base = read_revision(merge_base)?;
  let head = match head {
    Some(revision) => read_revision(revision)?,
    None => match std::fs::read_to_string(cwd.join(path)) {
      Ok(text) => text,
      Err(e) => {
        debug!("Could not read {ROOT_TSCONFIG_FILE} ({e}); not narrowing");
        return None;
      }
    },
  };
  let aliases = changed_aliases(&base, &head);
  if aliases.is_none() {
    debug!("{ROOT_TSCONFIG_FILE} changed more than compilerOptions.paths; not narrowing");
  }
  aliases
}

/// Files the analyzer did not parse that mention a changed alias, relative to
/// `cwd`, each with the aliases it mentions. Covers whatever lives in a project
/// root outside its analyzed source files: `jest.config.ts`, `.storybook/`,
/// e2e folders, `.vue` files. `.gitignore` is honoured; files over 2 MiB and
/// files that are not UTF-8 are skipped.
pub fn files_mentioning<'a>(
  cwd: &Path,
  project_roots: impl IntoIterator<Item = &'a Path>,
  aliases: &'a [AliasPattern],
  analyzed: impl Fn(&Path) -> bool,
) -> Vec<(PathBuf, Vec<&'a AliasPattern>)> {
  const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

  let roots: BTreeSet<&Path> = project_roots.into_iter().collect();
  let mut seen = BTreeSet::new();
  let mut found = Vec::new();
  for root in roots {
    let walker = ignore::WalkBuilder::new(cwd.join(root))
      .hidden(false)
      .require_git(false)
      .build();
    for entry in walker.flatten() {
      if !entry.file_type().is_some_and(|t| t.is_file()) {
        continue;
      }
      let Ok(relative) = entry.path().strip_prefix(cwd) else {
        continue;
      };
      if analyzed(relative) || !seen.insert(relative.to_path_buf()) {
        continue;
      }
      if entry.metadata().map_or(true, |m| m.len() > MAX_FILE_BYTES) {
        continue;
      }
      let Ok(text) = std::fs::read_to_string(entry.path()) else {
        continue;
      };
      let mentioned: Vec<_> = aliases.iter().filter(|a| a.mentioned_in(&text)).collect();
      if !mentioned.is_empty() {
        found.push((relative.to_path_buf(), mentioned));
      }
    }
  }
  found
}

/// tsconfig is JSONC: strip a byte-order mark, comments and trailing commas,
/// then parse.
fn parse_jsonc(text: &str) -> Option<Value> {
  let mut text = text.trim_start_matches('\u{feff}').to_string();
  json_strip_comments::strip(&mut text).ok()?;
  serde_json::from_str(&text).ok()
}

/// Remove `compilerOptions.paths` from `config` and return it (`Some(None)`
/// when absent), so the rest can be compared. `None` when the file does not
/// have the shape tsconfig requires.
fn take_paths(config: &mut Value) -> Option<Option<Map<String, Value>>> {
  let config = config.as_object_mut()?;
  let Some(options) = config.get_mut("compilerOptions") else {
    return Some(None);
  };
  let options = options.as_object_mut()?;
  let paths = match options.remove("paths") {
    None => None,
    Some(Value::Object(paths)) => Some(paths),
    Some(_) => return None,
  };
  // `{ "compilerOptions": { "paths": {...} } }` and `{}` differ only in paths.
  if options.is_empty() {
    config.remove("compilerOptions");
  }
  Some(paths)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn aliases(base: &str, head: &str) -> Option<Vec<String>> {
    changed_aliases(base, head).map(|patterns| patterns.into_iter().map(|p| p.alias).collect())
  }

  #[test]
  fn reports_added_removed_and_retargeted_aliases() {
    let base = r#"{ "compilerOptions": { "paths": {
      "@a": ["a.ts"], "@b": ["b.ts"], "@same": ["s.ts"] } } }"#;
    let head = r#"{ "compilerOptions": { "paths": {
      "@a": ["a2.ts"], "@c": ["c.ts"], "@same": ["s.ts"] } } }"#;

    assert_eq!(
      aliases(base, head),
      Some(vec!["@a".to_string(), "@b".to_string(), "@c".to_string()])
    );
  }

  #[test]
  fn an_unchanged_paths_map_narrows_to_nothing() {
    let base = r#"{ "compilerOptions": { "paths": { "@a": ["a.ts"] } } }"#;
    let head = "// reformatted\n{\n  \"compilerOptions\": {\n    \"paths\": { \"@a\": [\"a.ts\"], },\n  },\n}\n";

    assert_eq!(aliases(base, head), Some(vec![]));
  }

  #[test]
  fn adding_the_first_paths_map_narrows() {
    let head = r#"{ "compilerOptions": { "paths": { "@a": ["a.ts"] } } }"#;

    assert_eq!(aliases("{}", head), Some(vec!["@a".to_string()]));
  }

  #[test]
  fn any_other_difference_is_not_narrowed() {
    let paths = r#""paths": { "@a": ["a.ts"] }"#;
    let base = format!(r#"{{ "compilerOptions": {{ {paths} }} }}"#);
    for other in [
      r#""strict": true,"#,
      r#""baseUrl": "src","#,
      r#""paths": 1, "x": 1,"#,
    ] {
      let head = format!(r#"{{ "compilerOptions": {{ {other} {paths} }} }}"#);
      assert_eq!(aliases(&base, &head), None, "{other}");
    }
    let extends = format!(r#"{{ "extends": "./other.json", "compilerOptions": {{ {paths} }} }}"#);
    assert_eq!(aliases(&base, &extends), None);
  }

  #[test]
  fn extends_with_paths_appearing_or_vanishing_is_not_narrowed() {
    let without = r#"{ "extends": "./parent.json" }"#;
    let with =
      r#"{ "extends": "./parent.json", "compilerOptions": { "paths": { "@a": ["a.ts"] } } }"#;

    // The child's paths replace the inherited ones wholesale, so either
    // direction swaps aliases this diff cannot see.
    assert_eq!(aliases(without, with), None);
    assert_eq!(aliases(with, without), None);
  }

  #[test]
  fn extends_with_paths_on_both_sides_narrows() {
    let base =
      r#"{ "extends": "./parent.json", "compilerOptions": { "paths": { "@a": ["a.ts"] } } }"#;
    let head =
      r#"{ "extends": "./parent.json", "compilerOptions": { "paths": { "@a": ["b.ts"] } } }"#;

    assert_eq!(aliases(base, head), Some(vec!["@a".to_string()]));
  }

  #[test]
  fn a_byte_order_mark_still_parses() {
    let base = "\u{feff}{ \"compilerOptions\": { \"paths\": { \"@a\": [\"a.ts\"] } } }";
    let head = "\u{feff}{ \"compilerOptions\": { \"paths\": { \"@a\": [\"b.ts\"] } } }";

    assert_eq!(aliases(base, head), Some(vec!["@a".to_string()]));
  }

  #[test]
  fn unparseable_or_unmodelled_input_is_not_narrowed() {
    let ok = r#"{ "compilerOptions": { "paths": {} } }"#;
    assert_eq!(aliases(ok, "{ not json"), None);
    assert_eq!(aliases(ok, "[]"), None);
    assert_eq!(
      aliases(ok, r#"{ "compilerOptions": { "paths": [] } }"#),
      None
    );
    for key in ["*", "@acme/*-utils", "@acme/*/x/*"] {
      let head = format!(r#"{{ "compilerOptions": {{ "paths": {{ "{key}": ["x"] }} }} }}"#);
      assert_eq!(aliases(ok, &head), None, "{key}");
    }
  }

  #[test]
  fn exact_alias_matches_only_its_own_specifier() {
    let pattern = AliasPattern::new("@acme/a").unwrap();

    assert!(pattern.matches("@acme/a"));
    assert!(!pattern.matches("@acme/a/sub"));
    assert!(!pattern.matches("@acme/ab"));
    assert!(pattern.mentioned_in("import { a } from '@acme/a';"));
    assert!(pattern.mentioned_in("const a = require(\"@acme/a\");"));
    assert!(pattern.mentioned_in("const a = await import(`@acme/a`);"));
    assert!(!pattern.mentioned_in("import '@acme/a/sub';"));
    assert!(!pattern.mentioned_in("import '@acme/ab';"));
  }

  #[test]
  fn wildcard_alias_matches_everything_under_its_prefix() {
    let pattern = AliasPattern::new("@acme/a/*").unwrap();

    assert!(pattern.matches("@acme/a/sub"));
    assert!(pattern.matches("@acme/a/sub/deep"));
    assert!(!pattern.matches("@acme/a"));
    assert!(!pattern.matches("@acme/a/"));
    assert!(pattern.mentioned_in("import '@acme/a/polyfill';"));
    assert!(!pattern.mentioned_in("import '@acme/a';"));
    assert!(!pattern.mentioned_in("import '@acme/a/';"));
  }

  #[test]
  fn regex_characters_in_an_alias_are_literal() {
    let pattern = AliasPattern::new("@acme/a.b").unwrap();

    assert!(pattern.mentioned_in("import '@acme/a.b';"));
    assert!(!pattern.mentioned_in("import '@acme/aXb';"));
  }
}
