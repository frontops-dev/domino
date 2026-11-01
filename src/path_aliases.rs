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
use std::path::{Component, Path, PathBuf};
use tracing::debug;

/// How much of a file is read to reject one that is not UTF-8 before reading
/// the rest.
const CHUNK_BYTES: usize = 8 * 1024;

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

  /// The literal text every mention contains: the whole key, or for a
  /// `prefix/*` key everything up to and including `/`.
  pub fn prefix(&self) -> &str {
    &self.prefix
  }

  /// Whether `source` has a string literal holding a matching specifier. This
  /// finds the forms the analyzer records no import for: side-effect imports,
  /// `require()` and non-literal `import()` arguments.
  pub fn mentioned_in(&self, source: &str) -> bool {
    // The substring check is a cheap filter: most files never mention the alias.
    source.contains(self.prefix.as_str()) && self.quoted.is_match(source)
  }

  /// Byte offsets of the string literals [`Self::mentioned_in`] finds, each at
  /// its opening quote (where the parser's span for that literal starts).
  pub fn mention_offsets<'s>(&'s self, source: &'s str) -> impl Iterator<Item = usize> + 's {
    self.quoted.find_iter(source).map(|m| m.start())
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
/// e2e folders, `.vue` files. `.gitignore` is honoured, `.git` and
/// `node_modules` are never entered, and files over 2 MiB and files whose
/// first [`CHUNK_BYTES`] are not UTF-8 are skipped. Most files in a project
/// root are images, fonts and other files that cannot hold a mention, so a
/// file is rejected as soon as its first chunk is not UTF-8, and only read in
/// full otherwise. Past that chunk, invalid bytes are replaced rather than
/// rejected: a stray latin-1 byte must not hide a mention, and a miss costs
/// more than an over-selection.
///
/// `projects` are `(root, sourceRoot)` pairs. A project is scanned through
/// its root, except a workspace-root project (root `""`, `"."` or `"./"`):
/// walking that would read the whole repository, and every nested project is
/// scanned through its own root anyway. Such a project is scanned through its
/// sourceRoot instead, unless that is the workspace root too (see the skip
/// below).
pub fn files_mentioning<'a>(
  cwd: &Path,
  projects: impl IntoIterator<Item = (&'a Path, &'a Path)>,
  aliases: &'a [AliasPattern],
  analyzed: impl Fn(&Path) -> bool,
) -> Vec<(PathBuf, Vec<&'a AliasPattern>)> {
  const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

  // Never walk the workspace root itself. A hit there is owned like a direct
  // edit to that file, and outside a workspace-root project's sourceRoot that
  // never adds a project:
  // - one with a sourceRoot (`src`) owns no file outside it, since
  //   ProjectIndex keeps workspace roots out of its root fallback, so only its
  //   sourceRoot is walked (a project nested there is walked twice, but each
  //   file is reported once and ProjectIndex attributes it);
  // - one with sourceRoot `.` owns nothing, as no relative path starts with
  //   a `.` component;
  // - one without a sourceRoot (`""`) owns every path, so it already owns the
  //   changed `tsconfig.base.json` that put us here.
  let roots: BTreeSet<&Path> = projects
    .into_iter()
    .map(|(root, source_root)| {
      if is_workspace_root(root) {
        source_root
      } else {
        root
      }
    })
    .filter(|dir| !is_workspace_root(dir))
    .collect();
  if aliases.is_empty() {
    return Vec::new();
  }
  // Every mention contains its alias's prefix, so one byte search for any of
  // them settles most files without decoding them.
  let any_prefix = aliases
    .iter()
    .map(|a| regex::escape(a.prefix()))
    .collect::<Vec<_>>()
    .join("|");
  let Ok(any_prefix) = regex::bytes::Regex::new(&any_prefix) else {
    return Vec::new();
  };
  let mut seen = BTreeSet::new();
  let mut found = Vec::new();
  for root in roots {
    // Hidden entries stay in (`.storybook/`), but `.git` holds git's own
    // data, not project files, and the ignore crate only skips it while
    // hidden entries are skipped. `node_modules` is third-party code, pruned
    // even where it is not gitignored, as the analyzer prunes it.
    let walker = ignore::WalkBuilder::new(cwd.join(root))
      .hidden(false)
      .require_git(false)
      .filter_entry(|entry| !matches!(entry.file_name().to_str(), Some(".git" | "node_modules")))
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
      let Some(len) = entry
        .metadata()
        .ok()
        .map(|m| m.len())
        .filter(|&len| len <= MAX_FILE_BYTES)
      else {
        continue;
      };
      let Some(bytes) = read_if_utf8_start(entry.path(), len) else {
        continue;
      };
      if !any_prefix.is_match(&bytes) {
        continue;
      }
      let text = String::from_utf8_lossy(&bytes);
      let mentioned: Vec<_> = aliases.iter().filter(|a| a.mentioned_in(&text)).collect();
      if !mentioned.is_empty() {
        found.push((relative.to_path_buf(), mentioned));
      }
    }
  }
  found
}

/// Whether `path` names the workspace root: `""`, `"."` or `"./"`.
fn is_workspace_root(path: &Path) -> bool {
  path.components().all(|c| c == Component::CurDir)
}

/// The contents of `path` (`len` bytes long), or `None` when it cannot be
/// read or its first [`CHUNK_BYTES`] are not UTF-8. The rest is not validated
/// here. Only the first chunk is allocated until it passes, so a large binary
/// is rejected without allocating its whole length.
fn read_if_utf8_start(path: &Path, len: u64) -> Option<Vec<u8>> {
  use std::io::Read;

  let len = usize::try_from(len).ok()?;
  let mut file = std::fs::File::open(path).ok()?;
  let mut bytes = Vec::with_capacity(CHUNK_BYTES.min(len.saturating_add(1)));
  (&mut file)
    .take(CHUNK_BYTES as u64)
    .read_to_end(&mut bytes)
    .ok()?;
  // An error with no `error_len` is a character cut off at the chunk end,
  // which the rest of the file may complete.
  if std::str::from_utf8(&bytes).is_err_and(|e| e.error_len().is_some()) {
    return None;
  }
  // `len` comes from metadata read earlier, so the file may have changed
  // since; it only sizes the allocation. The extra byte spares the final read
  // that finds the end of the file a reallocation.
  bytes.reserve(len.saturating_sub(bytes.len()).saturating_add(1));
  file.read_to_end(&mut bytes).ok()?;
  Some(bytes)
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
  fn mention_offsets_point_at_each_opening_quote() {
    let pattern = AliasPattern::new("@acme/a").unwrap();
    let source = "import { a } from '@acme/a';\nconst b = require(\"@acme/a\");\n";

    let offsets: Vec<usize> = pattern.mention_offsets(source).collect();
    assert_eq!(offsets, vec![18, 47]);
    assert!(offsets
      .iter()
      .all(|&o| source[o..].starts_with(['\'', '"'])));
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

  /// Writes `files` under a fresh workspace and returns the paths
  /// [`files_mentioning`] reports for `@acme/a` when scanning projects whose
  /// sourceRoot is their root.
  fn mentioning(files: &[(&str, &str)], roots: &[&str]) -> Vec<String> {
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, c)| (*p, c.as_bytes())).collect();
    mentioning_bytes(&files, roots)
  }

  /// [`mentioning`] for file contents that need not be UTF-8.
  fn mentioning_bytes(files: &[(&str, &[u8])], roots: &[&str]) -> Vec<String> {
    let projects: Vec<(&str, &str)> = roots.iter().map(|root| (*root, *root)).collect();
    mentioning_in_projects(files, &projects)
  }

  /// [`mentioning_bytes`] for `(root, sourceRoot)` pairs.
  fn mentioning_in_projects(files: &[(&str, &[u8])], projects: &[(&str, &str)]) -> Vec<String> {
    let dir = tempfile::TempDir::new().unwrap();
    for (path, content) in files {
      let path = dir.path().join(path);
      std::fs::create_dir_all(path.parent().unwrap()).unwrap();
      std::fs::write(path, content).unwrap();
    }
    let aliases = vec![AliasPattern::new("@acme/a").unwrap()];
    let projects = projects
      .iter()
      .map(|(root, source_root)| (Path::new(*root), Path::new(*source_root)));
    let mut found: Vec<String> = files_mentioning(dir.path(), projects, &aliases, |path| {
      path.extension().is_some_and(|e| e == "ts") && path.starts_with("src")
    })
    .into_iter()
    .map(|(path, _)| path.to_string_lossy().replace('\\', "/"))
    .collect();
    found.sort();
    found
  }

  const MENTION: &str = "require('@acme/a');\n";

  #[test]
  fn a_workspace_root_project_is_not_scanned() {
    let files = [
      ("jest.config.ts", MENTION),
      ("src/main.ts", MENTION),
      ("libs/lib-x/jest.config.ts", MENTION),
      ("tools/script.js", MENTION),
    ];

    for root in [".", "", "./"] {
      assert_eq!(
        mentioning(&files, &[root]),
        Vec::<String>::new(),
        "root {root:?}"
      );
    }
    // A nested project's files are found through its own root.
    assert_eq!(
      mentioning(&files, &[".", "libs/lib-x"]),
      vec!["libs/lib-x/jest.config.ts"]
    );
  }

  #[test]
  fn a_workspace_root_project_is_scanned_through_its_source_root() {
    let files: [(&str, &[u8]); 5] = [
      ("jest.config.json", MENTION.as_bytes()),
      ("tools/script.js", MENTION.as_bytes()),
      ("src/App.vue", MENTION.as_bytes()),
      ("src/main.ts", MENTION.as_bytes()),
      ("src/libs/lib-z/jest.config.json", MENTION.as_bytes()),
    ];

    for root in [".", "", "./"] {
      assert_eq!(
        mentioning_in_projects(&files, &[(root, "src")]),
        vec!["src/App.vue", "src/libs/lib-z/jest.config.json"],
        "root {root:?}"
      );
      // A sourceRoot that is itself the workspace root is not walked either.
      assert_eq!(
        mentioning_in_projects(&files, &[(root, root)]),
        Vec::<String>::new(),
        "root {root:?}"
      );
    }
    // A project nested under the sourceRoot is walked twice, reported once.
    assert_eq!(
      mentioning_in_projects(
        &files,
        &[("", "src"), ("src/libs/lib-z", "src/libs/lib-z/src")]
      ),
      vec!["src/App.vue", "src/libs/lib-z/jest.config.json"]
    );
  }

  #[test]
  fn git_directories_are_never_scanned() {
    let files = [
      (".git/HEAD", MENTION),
      (".git/logs/HEAD", MENTION),
      ("libs/lib-x/.git/HEAD", MENTION),
      ("libs/lib-x/.storybook/main.js", MENTION),
    ];

    assert_eq!(
      mentioning(&files, &[".", "libs/lib-x"]),
      vec!["libs/lib-x/.storybook/main.js"]
    );
  }

  #[test]
  fn node_modules_is_never_scanned() {
    let files = [
      ("node_modules/dep/index.js", MENTION),
      ("libs/lib-x/node_modules/dep/index.js", MENTION),
      ("libs/lib-x/e2e/app.cy.js", MENTION),
    ];

    assert_eq!(
      mentioning(&files, &["libs/lib-x"]),
      vec!["libs/lib-x/e2e/app.cy.js"]
    );
  }

  #[test]
  fn binary_files_without_a_mention_are_skipped() {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend((0..=255u8).cycle().take(64 * 1024));
    let files: [(&str, &[u8]); 2] = [
      ("libs/lib-x/assets/logo.png", &png),
      ("libs/lib-x/jest.config.js", MENTION.as_bytes()),
    ];

    assert_eq!(
      mentioning_bytes(&files, &["libs/lib-x"]),
      vec!["libs/lib-x/jest.config.js"]
    );
  }

  #[test]
  fn only_a_file_whose_first_chunk_is_not_utf8_is_skipped() {
    let mut early = b"\xff\xfe".to_vec();
    early.extend_from_slice(MENTION.as_bytes());
    // Valid well past the first chunk read, invalid only at the end: a stray
    // latin-1 byte in a source file, which must not hide its mention.
    let mut late = MENTION.as_bytes().to_vec();
    late.extend(std::iter::repeat_n(b'x', 64 * 1024));
    late.push(0xff);
    let files: [(&str, &[u8]); 2] = [
      ("libs/lib-x/early.js", &early),
      ("libs/lib-x/late.js", &late),
    ];

    assert_eq!(
      mentioning_bytes(&files, &["libs/lib-x"]),
      vec!["libs/lib-x/late.js"]
    );
  }

  #[test]
  fn a_character_split_across_chunk_reads_is_still_utf8() {
    // Every boundary a 4-byte character can straddle near the chunk size.
    let files: Vec<(String, Vec<u8>)> = (0..4)
      .map(|shift| {
        let mut content = vec![b'x'; CHUNK_BYTES - shift];
        content.extend_from_slice("\u{1f600}".as_bytes());
        content.extend_from_slice(MENTION.as_bytes());
        (format!("libs/lib-x/file{shift}.js"), content)
      })
      .collect();
    let files: Vec<(&str, &[u8])> = files
      .iter()
      .map(|(p, c)| (p.as_str(), c.as_slice()))
      .collect();

    assert_eq!(
      mentioning_bytes(&files, &["libs/lib-x"]),
      (0..4)
        .map(|shift| format!("libs/lib-x/file{shift}.js"))
        .collect::<Vec<_>>()
    );
  }

  #[test]
  fn each_file_reports_only_the_aliases_it_mentions() {
    let dir = tempfile::TempDir::new().unwrap();
    let lib = dir.path().join("libs/lib-x");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("a.js"), "require('@acme/a');").unwrap();
    std::fs::write(lib.join("b.js"), "require('@acme/b/x');").unwrap();
    std::fs::write(lib.join("none.js"), "require('@acme/c');").unwrap();
    let aliases = vec![
      AliasPattern::new("@acme/a").unwrap(),
      AliasPattern::new("@acme/b/*").unwrap(),
    ];
    let lib_x = Path::new("libs/lib-x");

    let mut found: Vec<(String, Vec<&str>)> =
      files_mentioning(dir.path(), [(lib_x, lib_x)], &aliases, |_| false)
        .into_iter()
        .map(|(path, hits)| {
          (
            path.to_string_lossy().replace('\\', "/"),
            hits.iter().map(|a| a.alias.as_str()).collect(),
          )
        })
        .collect();
    found.sort();
    assert_eq!(
      found,
      vec![
        ("libs/lib-x/a.js".to_string(), vec!["@acme/a"]),
        ("libs/lib-x/b.js".to_string(), vec!["@acme/b/*"]),
      ]
    );
    assert!(files_mentioning(dir.path(), [(lib_x, lib_x)], &[], |_| false).is_empty());
  }

  #[test]
  fn regex_characters_in_an_alias_are_literal() {
    let pattern = AliasPattern::new("@acme/a.b").unwrap();

    assert!(pattern.mentioned_in("import '@acme/a.b';"));
    assert!(!pattern.mentioned_in("import '@acme/aXb';"));
  }
}
