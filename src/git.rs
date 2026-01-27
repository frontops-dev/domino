use crate::error::{DominoError, Result};
use crate::types::ChangedFile;
use regex::Regex;
use std::path::Path;
use std::process::Command;
use tracing::{debug, warn};

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

/// Get git diff output between a commit and the working tree
/// Using two-dot diff (no HEAD target) to include staged and unstaged changes,
/// matching traf's behavior exactly.
pub fn get_diff(repo_path: &Path, base: &str) -> Result<String> {
  let output = Command::new("git")
    .arg("diff")
    .arg(base)
    .arg("--unified=0")
    .arg("--relative")
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

  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Parse git diff output to extract changed files and line numbers
pub fn get_changed_files(repo_path: &Path, base: &str) -> Result<Vec<ChangedFile>> {
  debug!("Getting diff for base: {}", base);

  // First, find the merge base between base and HEAD
  // This ensures we only see changes from the current branch, not changes
  // from the base branch that happened after branching
  let merge_base = get_merge_base(repo_path, base, "HEAD")?;
  debug!("Merge base: {}", merge_base);

  // Then diff the merge base against the working tree (not HEAD)
  // This includes both committed and uncommitted changes, matching traf's behavior
  let diff = get_diff(repo_path, &merge_base)?;

  parse_diff(&diff)
}

/// Parse git diff output into ChangedFile structs
fn parse_diff(diff: &str) -> Result<Vec<ChangedFile>> {
  // Regex to extract file path: matches "a/path/to/file" between quotes or spaces
  let file_regex = Regex::new(r#"(?:["\s]a/)(.*)(?:["\s]b/)"#)
    .map_err(|e| DominoError::Parse(format!("Invalid file regex: {}", e)))?;

  // Regex to extract line numbers: matches "+<line_number>" in diff header
  let line_regex = Regex::new(r"@@ -.* \+(\d+)(?:,\d+)? @@")
    .map_err(|e| DominoError::Parse(format!("Invalid line regex: {}", e)))?;

  let changed_files: Vec<ChangedFile> = diff
    .split("diff --git")
    .skip(1) // Skip the first empty split
    .filter_map(|file_diff| {
      // Extract file path
      let file_path = file_regex
        .captures(file_diff)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().replace('"', "").trim().to_string())?;

      // Extract changed line numbers
      let changed_lines: Vec<usize> = line_regex
        .captures_iter(file_diff)
        .filter_map(|caps| caps.get(1))
        .filter_map(|m| m.as_str().parse::<usize>().ok())
        .collect();

      if changed_lines.is_empty() {
        debug!("No changed lines found for file: {}", file_path);
        return None;
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
    assert_eq!(result[1].changed_lines, vec![103]);
  }

  #[test]
  fn test_parse_diff_empty() {
    let diff = "";
    let result = parse_diff(diff).unwrap();
    assert_eq!(result.len(), 0);
  }
}
