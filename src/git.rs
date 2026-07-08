use crate::error::{DominoError, Result};
use crate::types::ChangedFile;
use regex::Regex;
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;
use tracing::{debug, warn};

static FILE_RE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r#"(?:["\s]a/)(.*)(?:["\s]b/)"#).expect("file regex is valid"));
/// Captures both sides of a hunk header `@@ -X,Y +Z,W @@`:
/// group 1 = old start `X`, group 2 = old count `Y` (optional),
/// group 3 = new start `Z`, group 4 = new count `W` (optional).
/// The old side is needed to recover symbols removed by pure-deletion hunks.
static LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").expect("line regex is valid")
});

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

/// Read the contents of a file as it existed at a given revision.
///
/// `path` is interpreted relative to `repo_path` (the `./` prefix makes git
/// resolve it against the working directory rather than the repo top-level),
/// matching the `--relative` paths produced by [`get_diff`]. Returns
/// `Ok(None)` when the file does not exist at that revision — e.g. a newly
/// added file, for which there is no base content to recover — so callers can
/// treat "no base" as "nothing to trace" without erroring.
pub fn get_file_at_revision(
  repo_path: &Path,
  revision: &str,
  path: &Path,
) -> Result<Option<String>> {
  let spec = format!("{}:./{}", revision, path.display());
  let output = Command::new("git")
    .args(["show", &spec])
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git show: {}", e)))?;

  if !output.status.success() {
    debug!(
      "git show {} returned non-zero (file likely absent at base); skipping",
      spec
    );
    return Ok(None);
  }

  Ok(Some(String::from_utf8(output.stdout).unwrap_or_else(|e| {
    String::from_utf8_lossy(e.as_bytes()).into_owned()
  })))
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

      // Extract line numbers from each hunk header `@@ -X,Y +Z,W @@`.
      //
      // A hunk that adds or modifies lines (`W >= 1`) expands to every new-side
      // line `Z..Z+W`, so symbols living mid-hunk — not just at the hunk's start
      // — are visible to the downstream AST lookup. When `,W` / `,Y` is omitted,
      // git's convention is a single line (count = 1).
      //
      // A pure-deletion hunk (`W == 0`) has no new-side line to look up: the
      // removed code is gone from the working tree. But the symbol that enclosed
      // it (an exported object losing a property, a `switch` losing a case, or an
      // entire top-level declaration) is still changed, and its dependents are
      // affected. We record the *old-side* lines `X..X+Y` in `deleted_lines`;
      // downstream (`core.rs`) re-parses the base revision at those lines to
      // recover the enclosing symbol and trace its references. Using the old side
      // — rather than anchoring to a surviving new-side line — is what makes
      // deleting a whole symbol resolvable and avoids mis-attributing the change
      // to whichever symbol now happens to occupy the deletion point.
      let mut changed_lines: Vec<usize> = Vec::new();
      let mut deleted_lines: Vec<usize> = Vec::new();
      let parse_group = |caps: &regex::Captures, idx: usize| -> Option<usize> {
        caps.get(idx).and_then(|m| {
          m.as_str()
            .parse()
            .inspect_err(|e| warn!("Failed to parse hunk field '{}': {}", m.as_str(), e))
            .ok()
        })
      };
      for caps in line_regex.captures_iter(file_diff) {
        let (Some(old_start), Some(new_start)) = (parse_group(&caps, 1), parse_group(&caps, 3))
        else {
          continue;
        };
        let old_count = parse_group(&caps, 2).unwrap_or(1);
        let new_count = parse_group(&caps, 4).unwrap_or(1);
        if new_count == 0 {
          deleted_lines.extend(old_start..old_start + old_count);
        } else {
          changed_lines.extend(new_start..new_start + new_count);
        }
      }

      if changed_lines.is_empty() {
        if is_rename_or_copy {
          changed_lines.push(1);
        } else if !deleted_lines.is_empty() {
          // Deletion-only file (every hunk is `+Z,0`). It has no new-side lines
          // to AST-lookup, but `deleted_lines` lets us recover the removed
          // symbols from the base revision downstream, and the file's owning
          // package is still marked affected because its path is present.
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
        deleted_lines,
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

  /// A pure-deletion hunk (`+Z,0`) records its *old-side* lines in
  /// `deleted_lines` (never in `changed_lines`), while a paired addition hunk
  /// contributes its new-side lines to `changed_lines`. Here the deletion at
  /// `-5,3` removed base lines 5, 6, 7 and the addition at `+21,2` added new
  /// lines 21, 22.
  #[test]
  fn test_parse_diff_deletion_hunk_records_old_side_alongside_addition() {
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
    assert_eq!(result[0].deleted_lines, vec![5, 6, 7]);
  }

  /// A file whose only hunks are deletions (`+Z,0`) is kept in the result with
  /// an empty `changed_lines` but the removed base-revision lines in
  /// `deleted_lines`. Downstream, those lines re-parse the base revision to
  /// recover the deleted symbol and trace its dependents — while the file's own
  /// package is still marked because its path is present. Previously such files
  /// were kept with empty `changed_lines` and nothing else, so the reference
  /// cascade was skipped and dependents were under-reported.
  #[test]
  fn test_parse_diff_deletion_only_file_records_deleted_lines() {
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
    assert_eq!(result[0].deleted_lines, vec![5, 6, 7]);
  }

  /// Symmetric hunk header — old and new sides both carry a count. Common in
  /// diffs with non-zero context (`--unified=N` for N > 0) or when an edit
  /// replaces a block with another block of the same size. The greedy `.*`
  /// in `LINE_RE` already handles this; the test locks it in against future
  /// regex tweaks.
  #[test]
  fn test_parse_diff_symmetric_hunk_with_old_and_new_counts() {
    let diff = r#"diff --git a/src/foo.ts b/src/foo.ts
index 1234567..abcdefg 100644
--- a/src/foo.ts
+++ b/src/foo.ts
@@ -3,3 +3,3 @@ import { helper } from './helper.js';
-old line 3
-old line 4
-old line 5
+new line 3
+new line 4
+new line 5
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].changed_lines, vec![3, 4, 5]);
  }

  /// Regression for the deletion-only under-detection bug.
  ///
  /// `git diff --unified=0` emits a `+Z,0` hunk when a line is removed. The
  /// removed content no longer exists in the new file, so it contributes
  /// nothing to `changed_lines`; instead its *old-side* line (the shorthand
  /// `-495` = base line 495) is recorded in `deleted_lines`. Downstream, that
  /// line re-parses the base revision, resolves the enclosing top-level symbol
  /// (here the exported `declarativeCalculations` object), and traces every
  /// project that consumes it.
  ///
  /// Before the fix, deletion hunks contributed nothing at all, so symbol
  /// extraction found nothing and the reference cascade was skipped,
  /// under-reporting the deleted symbol's dependents.
  #[test]
  fn test_parse_diff_deletion_only_hunk_records_old_side_line() {
    let diff = r#"diff --git a/src/attrs.ts b/src/attrs.ts
index 1234567..abcdefg 100644
--- a/src/attrs.ts
+++ b/src/attrs.ts
@@ -495 +494,0 @@ export const declarativeCalculations = {
-  'Commissions & Fees': { alternative: ['Bank Charges'] },
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].changed_lines.is_empty());
    assert_eq!(result[0].deleted_lines, vec![495]);
  }

  /// Deleting an entire multi-line top-level symbol at the very top of a file
  /// produces `@@ -1,N +0,0 @@`. The whole old-side range must land in
  /// `deleted_lines` so the base revision can be re-parsed to recover the
  /// deleted declaration — the case the earlier new-side "anchor" heuristic
  /// could not handle (it would resolve whichever symbol now occupies line 1).
  #[test]
  fn test_parse_diff_deletion_of_leading_symbol_records_full_old_range() {
    let diff = r#"diff --git a/src/foo.ts b/src/foo.ts
index 1234567..abcdefg 100644
--- a/src/foo.ts
+++ b/src/foo.ts
@@ -1,3 +0,0 @@
-export const Removed = {
-  value: 1,
-};
"#;

    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].changed_lines.is_empty());
    assert_eq!(result[0].deleted_lines, vec![1, 2, 3]);
  }
}
