use crate::error::{DominoError, Result};
use crate::types::ChangedFile;
use regex::Regex;
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;
use tracing::{debug, warn};

static FILE_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#"(?:["\s]a/)(.*)(?:["\s]b/)"#).expect("file regex is valid"));
static LINE_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"@@ -.* \+(\d+)(?:,(\d+))? @@").expect("line regex is valid"));

/// Detect the default branch (tries origin/main, then origin/master)
pub fn detect_default_branch(repo_path: &Path) -> String {
  // Try origin/main first
  if Command::new("git")
    .args(["rev-parse", "--verify", "origin/main"])
    .current_dir(repo_path)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
  {
    return "origin/main".to_string();
  }

  // Fallback to origin/master
  if Command::new("git")
    .args(["rev-parse", "--verify", "origin/master"])
    .current_dir(repo_path)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
  {
    return "origin/master".to_string();
  }

  // Default fallback
  "origin/main".to_string()
}

/// Resolve a git ref to its SHA
fn resolve_ref(repo_path: &Path, reference: &str) -> Result<String> {
  let output = Command::new("git")
    .args(["rev-parse", reference])
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git rev-parse: {}", e)))?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    return Err(DominoError::Other(format!(
      "Git rev-parse failed for '{}': {}",
      reference, stderr
    )));
  }

  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get the merge base between two branches
pub fn get_merge_base(repo_path: &Path, base: &str, head: &str) -> Result<String> {
  // Try git merge-base first
  let output = Command::new("git")
    .args(["merge-base", base, head])
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git merge-base: {}", e)))?;

  if output.status.success() {
    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !oid.is_empty() {
      return Ok(oid);
    }
  }

  // Fallback to using the base ref directly
  debug!("Falling back to using base ref directly");
  let output = Command::new("git")
    .args(["rev-parse", base])
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git rev-parse: {}", e)))?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    return Err(DominoError::Other(format!(
      "Git rev-parse failed for '{}': {}",
      base, stderr
    )));
  }

  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get git diff output between a commit and a head ref or the working tree.
///
/// When `head` is `Some(h)`, performs a two-dot diff between `base` and `h`
/// (commit-to-commit). When `head` is `None`, diffs `base` against the working
/// tree (staged and unstaged changes included), matching traf's behavior.
pub fn get_diff(repo_path: &Path, base: &str, head: Option<&str>) -> Result<String> {
  let mut cmd = Command::new("git");
  cmd.arg("diff");

  if let Some(h) = head {
    cmd.arg(format!("{}..{}", base, h));
  } else {
    cmd.arg(base);
  }

  cmd.arg("--unified=0").arg("--relative");

  let output = cmd
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git diff: {}", e)))?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    return Err(DominoError::Other(format!(
      "Git diff failed for base '{}': {}",
      base, stderr
    )));
  }

  Ok(
    String::from_utf8(output.stdout)
      .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
  )
}

/// Parse git diff output to extract changed files and line numbers.
/// Returns the changed files along with the computed merge-base SHA.
///
/// When `head` is `Some(h)`, the diff is computed as `base..head`
/// (commit-to-commit, no merge-base computation). When `head` is `None`,
/// the diff is computed between `merge-base(base, HEAD)` and the working tree.
pub fn get_changed_files(
  repo_path: &Path,
  base: &str,
  head: Option<&str>,
) -> Result<(Vec<ChangedFile>, String)> {
  debug!("Getting diff for base: {}", base);

  let (diff_base, merge_base) = if head.is_some() {
    debug!("Explicit head provided, using base ref directly");
    let resolved = resolve_ref(repo_path, base)?;
    (resolved.clone(), resolved)
  } else {
    let mb = get_merge_base(repo_path, base, "HEAD")?;
    debug!("Merge base: {}", mb);
    (mb.clone(), mb)
  };

  let diff = get_diff(repo_path, &diff_base, head)?;
  let files = parse_diff(&diff)?;

  Ok((files, merge_base))
}

/// Parse git diff output into ChangedFile structs
fn parse_diff(diff: &str) -> Result<Vec<ChangedFile>> {
  let file_regex = &*FILE_RE;
  let line_regex = &*LINE_RE;

  let changed_files: Vec<ChangedFile> = diff
    .split("diff --git")
    .skip(1) // Skip the first empty split
    .filter_map(|file_diff| {
      // Extract file path (from the "a/" side of the diff header)
      let file_path = file_regex
        .captures(file_diff)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().replace('"', "").trim().to_string())?;

      // For renamed/copied files, use the new path instead of the old path.
      let new_path = file_diff
        .lines()
        .find(|line| line.starts_with("rename to ") || line.starts_with("copy to "))
        .map(|line| {
          line
            .trim_start_matches("rename to ")
            .trim_start_matches("copy to ")
            .trim()
            .to_string()
        });
      let is_rename_or_copy = new_path.is_some();
      let file_path = new_path.unwrap_or(file_path);

      // Extract changed line numbers. For each hunk header `@@ -X,Y +Z,W @@`
      // expand to every line in the new-side range `Z..Z+W`, so symbols that
      // live mid-hunk (not just at the hunk's starting line) are visible to
      // downstream AST lookups. When `,W` is omitted, git's convention is a
      // single-line hunk (count = 1). Pure deletion hunks (`W == 0`) produce
      // an empty range — see the `has_hunks` branch below for how those are
      // preserved.
      let ranges: Vec<std::ops::Range<usize>> = line_regex
        .captures_iter(file_diff)
        .filter_map(|caps| {
          let start: usize = caps.get(1)?.as_str().parse().ok()?;
          let count: usize = caps
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(1);
          Some(start..start + count)
        })
        .collect();
      let has_hunks = !ranges.is_empty();
      let mut changed_lines: Vec<usize> = ranges.into_iter().flatten().collect();

      if changed_lines.is_empty() {
        if is_rename_or_copy {
          changed_lines.push(1);
        } else if has_hunks {
          // Deletion-only file (every hunk is `+Z,0`). Keep the file entry
          // with no line numbers so its owning package is still marked as
          // affected — deleting an exported symbol is a real change even
          // though there's nothing in the new file to AST-lookup.
          debug!("Only deletion hunks for file: {}", file_path);
        } else if file_diff
          .lines()
          .any(|line| line.starts_with("Binary files"))
        {
          debug!("Binary file detected: {}", file_path);
        } else {
          debug!("No changed lines found for file: {}", file_path);
          return None;
        }
      }

      Some(ChangedFile {
        file_path: file_path.into(),
        changed_lines,
      })
    })
    .collect();

  if changed_files.is_empty() {
    warn!("No changed files found in diff");
  } else {
    debug!("Found {} changed files", changed_files.len());
  }

  Ok(changed_files)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_diff() {
    let diff = r#"diff --git a/libs/core/src/utils.ts b/libs/core/src/utils.ts
index 1234567..abcdefg 100644
--- a/libs/core/src/utils.ts
+++ b/libs/core/src/utils.ts
@@ -15,0 +16,1 @@ export function findRootNode() {
+  return node.getParent();
@@ -45,1 +46,1 @@ export function getPackageName() {
-  return projects.find(p => p.path === path);
+  return projects.find(({ sourceRoot }) => path.includes(sourceRoot));
diff --git a/libs/nx/src/cli.ts b/libs/nx/src/cli.ts
index 9876543..fedcba9 100644
--- a/libs/nx/src/cli.ts
+++ b/libs/nx/src/cli.ts
@@ -102,0 +103,2 @@ export async function run(): Promise<void> {
+  // New code
+  console.log('test');
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 2);

    assert_eq!(
      result[0].file_path.to_str().unwrap(),
      "libs/core/src/utils.ts"
    );
    assert_eq!(result[0].changed_lines, vec![16, 46]);

    assert_eq!(result[1].file_path.to_str().unwrap(), "libs/nx/src/cli.ts");
    assert_eq!(result[1].changed_lines, vec![103, 104]);
  }

  #[test]
  fn test_parse_diff_empty() {
    let diff = "";
    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 0);
  }

  #[test]
  fn test_parse_diff_renamed_file() {
    let diff = r#"diff --git a/libs/old-dir/provider.ts b/libs/new-dir/provider.ts
similarity index 95%
rename from libs/old-dir/provider.ts
rename to libs/new-dir/provider.ts
index 1234567..abcdefg 100644
--- a/libs/old-dir/provider.ts
+++ b/libs/new-dir/provider.ts
@@ -10,1 +10,1 @@ export class Provider {
-  return 'old';
+  return 'new';
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);

    // Should use the NEW path, not the old path
    assert_eq!(
      result[0].file_path.to_str().unwrap(),
      "libs/new-dir/provider.ts"
    );
    assert_eq!(result[0].changed_lines, vec![10]);
  }

  #[test]
  fn test_parse_diff_renamed_file_with_changes() {
    // A rename that also has content changes in multiple hunks
    let diff = r#"diff --git a/src/quotes/helper.ts b/src/quote-page/helper.ts
similarity index 80%
rename from src/quotes/helper.ts
rename to src/quote-page/helper.ts
index 1234567..abcdefg 100644
--- a/src/quotes/helper.ts
+++ b/src/quote-page/helper.ts
@@ -5,1 +5,1 @@ export function getQuote() {
-  return fetchQuote();
+  return fetchPlatformicQuote();
@@ -20,0 +20,3 @@ export function formatQuote() {
+  // New validation logic
+  validateQuote();
+  return formatted;
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);

    // Should use the NEW path
    assert_eq!(
      result[0].file_path.to_str().unwrap(),
      "src/quote-page/helper.ts"
    );
    // Should have every line in each hunk's new-side range
    assert_eq!(result[0].changed_lines, vec![5, 20, 21, 22]);
  }

  #[test]
  fn test_parse_diff_mixed_renamed_and_normal() {
    // A diff with one renamed file and one normal file
    let diff = r#"diff --git a/src/old/component.ts b/src/new/component.ts
similarity index 90%
rename from src/old/component.ts
rename to src/new/component.ts
index 1234567..abcdefg 100644
--- a/src/old/component.ts
+++ b/src/new/component.ts
@@ -3,1 +3,1 @@
-  old code
+  new code
diff --git a/src/index.ts b/src/index.ts
index 9876543..fedcba9 100644
--- a/src/index.ts
+++ b/src/index.ts
@@ -1,1 +1,1 @@
-export { Component } from './old/component';
+export { Component } from './new/component';
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 2);

    // First file: renamed, should use new path
    assert_eq!(
      result[0].file_path.to_str().unwrap(),
      "src/new/component.ts"
    );

    // Second file: normal, should use the regular path
    assert_eq!(result[1].file_path.to_str().unwrap(), "src/index.ts");
  }

  #[test]
  fn test_parse_diff_rename_only() {
    let diff = r#"diff --git a/src/old/name.ts b/src/new/name.ts
similarity index 100%
rename from src/old/name.ts
rename to src/new/name.ts
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].file_path.to_str().unwrap(), "src/new/name.ts");
    assert_eq!(result[0].changed_lines, vec![1]);
  }

  #[test]
  fn test_parse_diff_binary_file() {
    let diff = r#"diff --git a/apps/e2e/src/__screenshots__/tests/visual.spec.ts/screenshot.png b/apps/e2e/src/__screenshots__/tests/visual.spec.ts/screenshot.png
index 1234567..abcdefg 100644
Binary files a/apps/e2e/src/__screenshots__/tests/visual.spec.ts/screenshot.png and b/apps/e2e/src/__screenshots__/tests/visual.spec.ts/screenshot.png differ
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(
      result[0].file_path.to_str().unwrap(),
      "apps/e2e/src/__screenshots__/tests/visual.spec.ts/screenshot.png"
    );
    assert!(result[0].changed_lines.is_empty());
  }

  #[test]
  fn test_parse_diff_new_binary_file() {
    let diff = r#"diff --git "a/image.png" "b/image.png"
new file mode 100644
index 000000000..26b848d67
Binary files /dev/null and "b/image.png" differ
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].file_path.to_str().unwrap(), "image.png");
    assert!(result[0].changed_lines.is_empty());
  }

  #[test]
  fn test_parse_diff_binary_mixed_with_source() {
    let diff = r#"diff --git a/apps/e2e/screenshot.png b/apps/e2e/screenshot.png
index 1234567..abcdefg 100644
Binary files a/apps/e2e/screenshot.png and b/apps/e2e/screenshot.png differ
diff --git a/libs/core/src/utils.ts b/libs/core/src/utils.ts
index 1234567..abcdefg 100644
--- a/libs/core/src/utils.ts
+++ b/libs/core/src/utils.ts
@@ -15,0 +16,1 @@ export function findRootNode() {
+  return node.getParent();
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 2);

    assert_eq!(
      result[0].file_path.to_str().unwrap(),
      "apps/e2e/screenshot.png"
    );
    assert!(result[0].changed_lines.is_empty());

    assert_eq!(
      result[1].file_path.to_str().unwrap(),
      "libs/core/src/utils.ts"
    );
    assert_eq!(result[1].changed_lines, vec![16]);
  }

  #[test]
  fn test_parse_diff_copy_only() {
    let diff = r#"diff --git a/src/original.ts b/src/copied.ts
similarity index 100%
copy from src/original.ts
copy to src/copied.ts
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].file_path.to_str().unwrap(), "src/copied.ts");
    assert_eq!(result[0].changed_lines, vec![1]);
  }

  /// Regression test for issue #62. A multi-line hunk must contribute every
  /// line in its new-side range, not only the starting line. Otherwise an
  /// exported symbol declared mid-hunk is invisible to `find_node_at_line`,
  /// reference traversal is skipped, and downstream consumers are silently
  /// dropped from the affected set.
  #[test]
  fn test_parse_diff_multi_line_hunk_covers_full_range() {
    let diff = r#"diff --git a/packages/package-a/src/foo.ts b/packages/package-a/src/foo.ts
index 1234567..abcdefg 100644
--- a/packages/package-a/src/foo.ts
+++ b/packages/package-a/src/foo.ts
@@ -3 +3,7 @@ import { helper } from './helper.js';
-export const foo = (x: number): number => helper(x) + 1;
+interface FooOptions {
+  offset: number;
+  multiplier: number;
+}
+
+export const foo = (x: number, options: FooOptions = { offset: 1, multiplier: 3 }): number =>
+  helper(x) * options.multiplier + options.offset;
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].changed_lines, vec![3, 4, 5, 6, 7, 8, 9]);
  }

  /// When the hunk header omits `,W` (single-line hunk), git emits
  /// `@@ -N +M @@` rather than `@@ -N +M,1 @@`. The regex's count group is
  /// optional and must fall back to 1 so these hunks still produce a single
  /// line number.
  #[test]
  fn test_parse_diff_shorthand_count_defaults_to_one() {
    let diff = r#"diff --git a/src/foo.ts b/src/foo.ts
index 1234567..abcdefg 100644
--- a/src/foo.ts
+++ b/src/foo.ts
@@ -1 +1 @@
-export const foo = 1;
+export const foo = 2;
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].changed_lines, vec![1]);
  }

  /// A pure-deletion hunk (`+Z,0`) has no new-side lines to scan and must
  /// contribute zero entries to `changed_lines`. Pair it with a normal hunk
  /// so the file-skip branch (which triggers on a fully empty result) is not
  /// what's actually being tested.
  #[test]
  fn test_parse_diff_pure_deletion_hunk_contributes_zero() {
    let diff = r#"diff --git a/src/foo.ts b/src/foo.ts
index 1234567..abcdefg 100644
--- a/src/foo.ts
+++ b/src/foo.ts
@@ -5,3 +5,0 @@ prefix
-  deleted one
-  deleted two
-  deleted three
@@ -20,0 +21,2 @@ suffix
+  added one
+  added two
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].changed_lines, vec![21, 22]);
  }

  /// A file whose only hunks are deletions (`+Z,0`) must still be kept in
  /// the result with an empty `changed_lines`. Dropping it would hide real
  /// source changes — deleting an exported symbol is a meaningful change
  /// even though there are no new-file lines to AST-lookup. Downstream in
  /// `core.rs`, the file's owning package is still marked affected because
  /// the file path is present.
  #[test]
  fn test_parse_diff_deletion_only_file_kept_with_empty_lines() {
    let diff = r#"diff --git a/src/foo.ts b/src/foo.ts
index 1234567..abcdefg 100644
--- a/src/foo.ts
+++ b/src/foo.ts
@@ -5,3 +5,0 @@ export function foo() {
-  deleted one
-  deleted two
-  deleted three
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].file_path.to_str().unwrap(), "src/foo.ts");
    assert!(result[0].changed_lines.is_empty());
  }
}
