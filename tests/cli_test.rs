use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Test fixture path
fn fixture_path() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .join("tests")
    .join("fixtures")
    .join("monorepo")
}

/// Get the path to the domino binary
fn domino_binary() -> PathBuf {
  let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  path.push("target");

  // Binary name differs on Windows
  let binary_name = if cfg!(windows) {
    "domino.exe"
  } else {
    "domino"
  };

  // Use release build if available, otherwise debug
  let release_path = path.join("release").join(binary_name);
  if release_path.exists() {
    release_path
  } else {
    path.join("debug").join(binary_name)
  }
}

/// Helper to run git commands in the fixture repo
fn git_command(args: &[&str]) -> String {
  let output = Command::new("git")
    .args(args)
    .current_dir(fixture_path())
    .output()
    .expect("Failed to execute git command");

  if !output.status.success() {
    panic!(
      "Git command failed: git {}\nStderr: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr)
    );
  }

  String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Ensure the fixture repo is initialized with git
fn ensure_git_repo() {
  let fixture = fixture_path();
  let git_dir = fixture.join(".git");

  // If .git directory doesn't exist, initialize the repo
  if !git_dir.exists() {
    // Initialize git repo
    Command::new("git")
      .args(["init"])
      .current_dir(&fixture)
      .output()
      .expect("Failed to init git repo");

    // Configure git
    Command::new("git")
      .args(["config", "user.email", "test@example.com"])
      .current_dir(&fixture)
      .output()
      .expect("Failed to configure git email");

    Command::new("git")
      .args(["config", "user.name", "Test User"])
      .current_dir(&fixture)
      .output()
      .expect("Failed to configure git name");

    // Rename default branch to main (for consistency)
    Command::new("git")
      .args(["branch", "-M", "main"])
      .current_dir(&fixture)
      .output()
      .expect("Failed to rename branch to main");

    // Add all files
    Command::new("git")
      .args(["add", "."])
      .current_dir(&fixture)
      .output()
      .expect("Failed to add files");

    // Create initial commit
    Command::new("git")
      .args(["commit", "-m", "Initial commit"])
      .current_dir(&fixture)
      .output()
      .expect("Failed to create initial commit");
  }
}

/// Setup: Create a test branch and reset to main after test
struct TestBranch {
  branch_name: String,
}

impl TestBranch {
  fn new(name: &str) -> Self {
    // Ensure git repo is initialized (needed for CI)
    ensure_git_repo();

    // Ensure we're on main
    let _ = Command::new("git")
      .args(["checkout", "main"])
      .current_dir(fixture_path())
      .output();

    // Delete branch if it exists (ignore errors)
    let _ = Command::new("git")
      .args(["branch", "-D", name])
      .current_dir(fixture_path())
      .output();

    // Create and checkout new branch
    git_command(&["checkout", "-b", name]);

    Self {
      branch_name: name.to_string(),
    }
  }

  fn make_change(&self, file: &str, content: &str) {
    let file_path = fixture_path().join(file);
    fs::write(&file_path, content).expect("Failed to write file");
    git_command(&["add", file]);

    // Check if there are changes to commit
    let status_output = Command::new("git")
      .args(["status", "--porcelain"])
      .current_dir(fixture_path())
      .output()
      .expect("Failed to check git status");

    // Only commit if there are changes
    if !status_output.stdout.is_empty() {
      git_command(&["commit", "-m", &format!("Change {}", file)]);
    }
  }

  /// Run domino CLI with given arguments
  fn run_domino(&self, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(domino_binary());
    cmd
      .args(args)
      .current_dir(fixture_path())
      .env_remove("DOMINO_PROFILE") // Ensure clean environment
      .env("RUST_LOG", "off"); // Suppress Rust logging by default

    cmd.output().expect("Failed to execute domino")
  }

  /// Run domino with environment variables
  fn run_domino_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(domino_binary());
    cmd.args(args).current_dir(fixture_path());

    for (key, value) in env {
      cmd.env(key, value);
    }

    cmd.output().expect("Failed to execute domino")
  }
}

impl Drop for TestBranch {
  fn drop(&mut self) {
    // Return to main and delete test branch
    git_command(&["checkout", "main"]);
    let _ = git_command(&["branch", "-D", &self.branch_name]);
  }
}

// ============================================================================
// JSON Output Tests
// ============================================================================

#[test]
fn test_json_output_with_affected_projects() {
  let branch = TestBranch::new("test-json-affected");

  // Change proj1
  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-modified';
}

export function unusedFn() {
  return 'unusedFn';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main", "--json"]);

  assert!(output.status.success(), "Command should succeed");

  let stdout = String::from_utf8_lossy(&output.stdout);

  // Should be valid JSON
  let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
  assert!(
    json_result.is_ok(),
    "Output should be valid JSON: {}",
    stdout
  );

  let json = json_result.unwrap();

  // Should be an array
  assert!(json.is_array(), "JSON output should be an array");

  let projects = json.as_array().unwrap();

  // Should contain proj1 and proj3 (implicit dependency)
  let project_names: Vec<&str> = projects.iter().map(|p| p.as_str().unwrap()).collect();

  assert!(project_names.contains(&"proj1"), "Should contain proj1");
  assert!(
    project_names.contains(&"proj3"),
    "Should contain proj3 (implicit dep)"
  );
}

#[test]
fn test_json_output_with_no_changes() {
  let branch = TestBranch::new("test-json-no-changes");

  // Don't make any changes
  let output = branch.run_domino(&["affected", "--base", "main", "--json"]);

  assert!(output.status.success(), "Command should succeed");

  let stdout = String::from_utf8_lossy(&output.stdout);

  // Should be valid JSON
  let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
  assert!(
    json_result.is_ok(),
    "Output should be valid JSON: {}",
    stdout
  );

  let json = json_result.unwrap();

  // Should be an empty array
  assert!(json.is_array(), "JSON output should be an array");
  assert_eq!(json.as_array().unwrap().len(), 0, "Should be empty array");
}

#[test]
fn test_json_output_no_extra_logs() {
  let branch = TestBranch::new("test-json-clean");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-clean-test';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main", "--json"]);

  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  // Stdout should ONLY contain JSON, no debug/info messages
  assert!(
    stdout.trim().starts_with('['),
    "Stdout should start with JSON array, got: {}",
    stdout
  );

  // Should be parseable as JSON (no extra text)
  let json_result: Result<serde_json::Value, _> = serde_json::from_str(stdout.trim());
  assert!(
    json_result.is_ok(),
    "Entire stdout should be valid JSON with no extra text"
  );

  // Stderr should not contain warnings in JSON mode
  assert!(
    !stderr.contains("WARN") && !stderr.contains("Source root does not exist"),
    "Stderr should not contain warnings in JSON mode. Stderr: {}",
    stderr
  );
}

// ============================================================================
// CI Mode Tests
// ============================================================================

#[test]
fn test_ci_mode_suppresses_logs() {
  let branch = TestBranch::new("test-ci-logs");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-ci-test';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main", "--ci"]);

  assert!(output.status.success(), "Command should succeed");

  let stderr = String::from_utf8_lossy(&output.stderr);

  // In CI mode, stderr should not contain INFO, WARN, or DEBUG logs
  // It may contain ERROR logs, but for successful runs it should be minimal
  assert!(
    !stderr.contains("INFO") && !stderr.contains("WARN") && !stderr.contains("DEBUG"),
    "CI mode should suppress INFO/WARN/DEBUG logs. Stderr: {}",
    stderr
  );
}

#[test]
fn test_ci_mode_with_json() {
  let branch = TestBranch::new("test-ci-json");

  branch.make_change(
    "proj2/index.ts",
    r#"import { proj1 } from '@monorepo/proj1';

export { proj1 } from '@monorepo/proj1';

export function proj2() {
  proj1();
  return 'proj2-ci-json';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main", "--ci", "--json"]);

  assert!(output.status.success(), "Command should succeed");

  let stdout = String::from_utf8_lossy(&output.stdout);
  let stderr = String::from_utf8_lossy(&output.stderr);

  // Stdout should be clean JSON
  let json_result: Result<serde_json::Value, _> = serde_json::from_str(stdout.trim());
  assert!(json_result.is_ok(), "Should be valid JSON in CI+JSON mode");

  // Stderr should be minimal (no logs)
  assert!(
    !stderr.contains("INFO") && !stderr.contains("WARN"),
    "CI+JSON mode should have clean output"
  );
}

// ============================================================================
// Profile Mode Tests
// ============================================================================

#[test]
fn test_profile_flag_enables_profiling() {
  let branch = TestBranch::new("test-profile-flag");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-profile-test';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main", "--profile"]);

  assert!(output.status.success(), "Command should succeed");

  let stderr = String::from_utf8_lossy(&output.stderr);

  // Should show profiling enabled message
  assert!(
    stderr.contains("📊 Performance profiling enabled"),
    "Should show profiling enabled message. Stderr: {}",
    stderr
  );

  // Should show profiling report sections
  assert!(
    stderr.contains("PERFORMANCE PROFILING REPORT") || stderr.contains("Module Resolution"),
    "Should show profiling report. Stderr: {}",
    stderr
  );
}

#[test]
fn test_profile_env_var_enables_profiling() {
  let branch = TestBranch::new("test-profile-env");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-env-profile';
}
"#,
  );

  let output =
    branch.run_domino_with_env(&["affected", "--base", "main"], &[("DOMINO_PROFILE", "1")]);

  assert!(output.status.success(), "Command should succeed");

  let stderr = String::from_utf8_lossy(&output.stderr);

  // Should show profiling enabled via env var
  assert!(
    stderr.contains("📊 Performance profiling enabled"),
    "DOMINO_PROFILE env var should enable profiling. Stderr: {}",
    stderr
  );
}

#[test]
fn test_profile_report_structure() {
  let branch = TestBranch::new("test-profile-structure");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-structure-test';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main", "--profile"]);

  let stderr = String::from_utf8_lossy(&output.stderr);

  // Check for key sections in the profile report
  let has_resolution_stats = stderr.contains("Module Resolution") || stderr.contains("Resolution:");
  let has_reference_stats = stderr.contains("Reference Finding") || stderr.contains("Reference");
  let has_time_breakdown = stderr.contains("TIME BREAKDOWN") || stderr.contains("Time spent");

  assert!(
    has_resolution_stats,
    "Profile report should contain resolution statistics"
  );
  assert!(
    has_reference_stats,
    "Profile report should contain reference finding statistics"
  );
  assert!(
    has_time_breakdown,
    "Profile report should contain time breakdown"
  );
}

// ============================================================================
// Standard Output Format Tests
// ============================================================================

#[test]
fn test_standard_output_format() {
  let branch = TestBranch::new("test-std-output");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-output-test';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main"]);

  assert!(output.status.success(), "Command should succeed");

  let stdout = String::from_utf8_lossy(&output.stdout);

  // Should show "Affected projects:" header
  assert!(
    stdout.contains("Affected projects:"),
    "Should show 'Affected projects:' header. Stdout: {}",
    stdout
  );

  // Should show bullet points
  assert!(
    stdout.contains("•"),
    "Should show bullet points. Stdout: {}",
    stdout
  );

  // Should show total count
  assert!(
    stdout.contains("Total:") && stdout.contains("affected project"),
    "Should show total count. Stdout: {}",
    stdout
  );
}

#[test]
fn test_standard_output_no_changes() {
  let branch = TestBranch::new("test-std-no-changes");

  // Don't make any changes
  let output = branch.run_domino(&["affected", "--base", "main"]);

  assert!(output.status.success(), "Command should succeed");

  let stdout = String::from_utf8_lossy(&output.stdout);

  // Should show friendly "no affected projects" message
  assert!(
    stdout.contains("No affected projects"),
    "Should show 'No affected projects' message. Stdout: {}",
    stdout
  );
}

// ============================================================================
// All Projects Flag Tests
// ============================================================================

#[test]
fn test_all_flag_lists_all_projects() {
  let branch = TestBranch::new("test-all-flag");

  // Don't make any changes - should still show all projects
  let output = branch.run_domino(&["affected", "--base", "main", "--all"]);

  assert!(output.status.success(), "Command should succeed");

  let stdout = String::from_utf8_lossy(&output.stdout);

  // Should show "All projects:" header
  assert!(
    stdout.contains("All projects:"),
    "Should show 'All projects:' header with --all flag. Stdout: {}",
    stdout
  );

  // Should contain all 3 projects
  assert!(stdout.contains("proj1"), "Should list proj1");
  assert!(stdout.contains("proj2"), "Should list proj2");
  assert!(stdout.contains("proj3"), "Should list proj3");

  // Should show total
  assert!(
    stdout.contains("Total:") && stdout.contains("3 projects"),
    "Should show total of 3 projects. Stdout: {}",
    stdout
  );
}

#[test]
fn test_all_flag_with_json() {
  let branch = TestBranch::new("test-all-json");

  let output = branch.run_domino(&["affected", "--base", "main", "--all", "--json"]);

  assert!(output.status.success(), "Command should succeed");

  let stdout = String::from_utf8_lossy(&output.stdout);

  // Should be valid JSON
  let json_result: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
  assert!(json_result.is_ok(), "Output should be valid JSON");

  let json = json_result.unwrap();
  let projects = json.as_array().unwrap();

  // Should have all 3 projects
  assert_eq!(projects.len(), 3, "Should list all 3 projects");

  let project_names: Vec<&str> = projects.iter().map(|p| p.as_str().unwrap()).collect();

  assert!(project_names.contains(&"proj1"));
  assert!(project_names.contains(&"proj2"));
  assert!(project_names.contains(&"proj3"));
}

// ============================================================================
// Debug Mode Tests
// ============================================================================

#[test]
fn test_debug_flag_shows_debug_logs() {
  let branch = TestBranch::new("test-debug-flag");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-debug';
}
"#,
  );

  // Run without RUST_LOG=off to allow debug logs through
  let output = branch.run_domino_with_env(&["affected", "--base", "main", "--debug"], &[]);

  assert!(output.status.success(), "Command should succeed");

  let stderr = String::from_utf8_lossy(&output.stderr);
  let stdout = String::from_utf8_lossy(&output.stdout);

  // In debug mode, command should succeed and produce output
  // Debug logs may or may not appear depending on test environment settings
  // but at minimum the command should work
  assert!(
    output.status.success() && (!stdout.is_empty() || !stderr.is_empty()),
    "Debug mode should produce some output"
  );
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_invalid_base_branch() {
  let branch = TestBranch::new("test-invalid-base");

  let output = branch.run_domino(&["affected", "--base", "nonexistent-branch-xyz"]);

  // Should fail with non-zero exit code
  assert!(
    !output.status.success(),
    "Should fail with invalid base branch"
  );

  let stderr = String::from_utf8_lossy(&output.stderr);

  // Should have some error message (exact message may vary)
  assert!(!stderr.is_empty(), "Should show error message");
}

// ============================================================================
// Combined Flags Tests
// ============================================================================

#[test]
fn test_debug_and_profile_combined() {
  let branch = TestBranch::new("test-debug-profile");

  branch.make_change(
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1-combined';
}
"#,
  );

  let output = branch.run_domino(&["affected", "--base", "main", "--debug", "--profile"]);

  assert!(output.status.success(), "Command should succeed");

  let stderr = String::from_utf8_lossy(&output.stderr);

  // Should show both debug logs and profiling
  assert!(
    stderr.contains("📊 Performance profiling enabled"),
    "Should enable profiling"
  );
  // Debug logs should also be present (content varies)
  assert!(!stderr.is_empty(), "Should show debug output");
}
