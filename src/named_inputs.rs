use crate::types::{ChangedFile, GlobalTrigger, Project};
use glob::Pattern;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

/// A compiled global-invalidation pattern that retains the name of the
/// `namedInput` it originated from (e.g. `sharedGlobals`), so the report can
/// surface a term the user wrote in their `nx.json`.
#[derive(Debug, Clone)]
pub struct GlobalPattern {
  pub named_input: String,
  pub raw_pattern: String,
  pub pattern: Pattern,
}

/// Resolved named inputs configuration from nx.json
#[derive(Debug, Default)]
pub struct ResolvedNamedInputs {
  /// Glob patterns for workspace-root files that invalidate all projects
  /// e.g., "babel.config.json", "patches/*"
  pub global_patterns: Vec<GlobalPattern>,
  /// Pre-compiled negation glob patterns for project-root files to exclude
  /// e.g., "**/*.figma.tsx"
  pub negation_patterns: Vec<Pattern>,
}

#[derive(Debug, Deserialize)]
struct NxJson {
  #[serde(default, rename = "namedInputs")]
  named_inputs: HashMap<String, Vec<serde_json::Value>>,
}

/// Parse and resolve namedInputs from nx.json.
/// Returns None if nx.json doesn't exist or has no namedInputs.
pub fn resolve_from_nx_json(cwd: &Path) -> Option<ResolvedNamedInputs> {
  let nx_json_path = cwd.join("nx.json");
  let content = fs::read_to_string(&nx_json_path).ok()?;
  let nx_json: NxJson = serde_json::from_str(&content).ok()?;

  if nx_json.named_inputs.is_empty() {
    debug!("No namedInputs found in nx.json");
    return None;
  }

  debug!(
    "Found {} named inputs in nx.json",
    nx_json.named_inputs.len()
  );

  // Resolve the "default" named input recursively, tracking the named-input
  // each pattern originated from so the report can show it back to the user.
  let mut resolved_patterns: Vec<(String, String)> = Vec::new();
  let mut visited = std::collections::HashSet::new();
  resolve_named_input(
    "default",
    "default",
    &nx_json.named_inputs,
    &mut resolved_patterns,
    &mut visited,
  );

  if resolved_patterns.is_empty() {
    debug!("No patterns resolved from namedInputs.default");
    return None;
  }

  let mut global_patterns: Vec<GlobalPattern> = Vec::new();
  let mut negation_patterns = Vec::new();

  for (origin, pattern_str) in &resolved_patterns {
    if let Some(negated) = pattern_str.strip_prefix('!') {
      // Negation pattern
      if let Some(suffix) = negated.strip_prefix("{projectRoot}/") {
        match Pattern::new(suffix) {
          Ok(pat) => {
            debug!("Negation pattern (project-root): !{}", suffix);
            negation_patterns.push(pat);
          }
          Err(e) => {
            warn!("Invalid negation glob pattern '{}': {}", suffix, e);
          }
        }
      } else if let Some(suffix) = negated.strip_prefix("{workspaceRoot}/") {
        debug!(
          "Negation pattern (workspace-root): !{} — skipping (not yet supported)",
          suffix
        );
      }
    } else if let Some(suffix) = pattern_str.strip_prefix("{workspaceRoot}/") {
      // Global workspace-root pattern
      match Pattern::new(suffix) {
        Ok(pat) => {
          debug!("Global pattern from '{}': {}", origin, suffix);
          global_patterns.push(GlobalPattern {
            named_input: origin.clone(),
            raw_pattern: pattern_str.clone(),
            pattern: pat,
          });
        }
        Err(e) => {
          warn!("Invalid glob pattern '{}': {}", suffix, e);
        }
      }
    }
    // {projectRoot}/** positive patterns are already handled by sourceRoot-based ownership
  }

  if global_patterns.is_empty() && negation_patterns.is_empty() {
    debug!("No actionable patterns resolved from namedInputs");
    return None;
  }

  debug!(
    "Resolved {} global patterns, {} negation patterns",
    global_patterns.len(),
    negation_patterns.len()
  );

  Some(ResolvedNamedInputs {
    global_patterns,
    negation_patterns,
  })
}

/// Label reported for turbo.json global-invalidation triggers. Mirrors the way
/// the Nx path reports the `namedInput` name (e.g. `sharedGlobals`): the term
/// surfaced back to the user is the key they actually wrote in their config.
pub(crate) const TURBO_GLOBAL_DEPENDENCIES: &str = "globalDependencies";

/// Resolve global-invalidation patterns from a root `turbo.json` / `turbo.jsonc`.
///
/// Turborepo's `globalDependencies` are the direct analogue of Nx's
/// `sharedGlobals`: workspace-root-relative globs whose contents feed the global
/// hash, so a change to any of them invalidates every task in the workspace.
/// They are compiled into the same [`ResolvedNamedInputs`] the Nx path produces,
/// so `core` treats both identically — including the dependency-manifest
/// exemption that keeps a lockfile in the list from short-circuiting to "all
/// projects" while lockfile analysis is enabled.
///
/// Turborepo has no per-project negation concept, so `negation_patterns` is
/// always empty. Returns `None` when there is no turbo config, it can't be
/// parsed, or it lists no usable `globalDependencies`.
pub fn resolve_from_turbo_json(cwd: &Path) -> Option<ResolvedNamedInputs> {
  let config = crate::workspace::turbo::parse_config(cwd)?;

  if config.global_dependencies.is_empty() {
    debug!("No globalDependencies found in turbo config");
    return None;
  }

  let mut global_patterns: Vec<GlobalPattern> = Vec::new();
  for raw_pattern in &config.global_dependencies {
    // Negations only carry meaning for task `inputs`/`outputs` (out of scope),
    // and Turborepo rejects globs that escape the workspace root. Skip both
    // rather than mis-compiling them into positive patterns that would
    // over-invalidate.
    if let Some(negated) = raw_pattern.strip_prefix('!') {
      // Dropping a negation silently over-includes files the user meant to
      // exclude, which changes the affected result — loud enough for warn!,
      // matching the root-escaping skip below.
      warn!(
        "Skipping negated globalDependency '!{}' (not supported)",
        negated
      );
      continue;
    }
    // A bare ".." or a "../..." prefix escapes the workspace root. Don't
    // reject a legitimate root-relative file that merely starts with two
    // literal dots, e.g. "..config.json".
    if raw_pattern.starts_with('/') || raw_pattern == ".." || raw_pattern.starts_with("../") {
      warn!(
        "Skipping globalDependency '{}': not relative to the workspace root",
        raw_pattern
      );
      continue;
    }

    // Strip a leading "./" before compiling: glob::Pattern matches it
    // literally, so "./.env" would never match git's relative path ".env" —
    // a silent no-op. `raw_pattern` is kept as-written for display.
    let compiled_pattern = raw_pattern.strip_prefix("./").unwrap_or(raw_pattern);

    match Pattern::new(compiled_pattern) {
      Ok(pattern) => {
        debug!("Global pattern from globalDependencies: {}", raw_pattern);
        global_patterns.push(GlobalPattern {
          named_input: TURBO_GLOBAL_DEPENDENCIES.to_string(),
          raw_pattern: raw_pattern.clone(),
          pattern,
        });
      }
      Err(e) => {
        warn!("Invalid glob pattern '{}': {}", raw_pattern, e);
      }
    }
  }

  if global_patterns.is_empty() {
    debug!("No actionable patterns resolved from globalDependencies");
    return None;
  }

  debug!(
    "Resolved {} global patterns from globalDependencies",
    global_patterns.len()
  );

  Some(ResolvedNamedInputs {
    global_patterns,
    negation_patterns: Vec::new(),
  })
}

/// Resolve the workspace's global-invalidation configuration, following the same
/// workspace-type precedence as project discovery
/// ([`crate::workspace::discover_projects`]): Nx before Turborepo.
///
/// A repo containing both `nx.json` and `turbo.json` is treated as an Nx
/// workspace, so its `namedInputs` are authoritative and turbo.json is ignored —
/// otherwise the projects and the global rules could come from two different
/// tools' configs.
pub fn resolve_global_inputs(cwd: &Path) -> Option<ResolvedNamedInputs> {
  if crate::workspace::nx::is_nx_workspace(cwd) {
    return resolve_from_nx_json(cwd);
  }
  if crate::workspace::turbo::is_turbo_workspace(cwd) {
    return resolve_from_turbo_json(cwd);
  }
  None
}

/// Recursively resolve a named input, following references to other named inputs.
///
/// `origin` is the user-facing namedInput name attributed to literal patterns
/// at this level (i.e. the most recent named input the user wrote in nx.json
/// along this recursion path). When `default` references `sharedGlobals`, the
/// recursive call passes `origin = "sharedGlobals"` so patterns surfaced from
/// there carry the term the user recognizes.
fn resolve_named_input(
  name: &str,
  origin: &str,
  all_inputs: &HashMap<String, Vec<serde_json::Value>>,
  resolved: &mut Vec<(String, String)>,
  visited: &mut std::collections::HashSet<String>,
) {
  if !visited.insert(name.to_string()) {
    debug!("Circular reference detected in namedInputs: {}", name);
    return;
  }

  let entries = match all_inputs.get(name) {
    Some(entries) => entries,
    None => {
      debug!("Named input '{}' not found in nx.json", name);
      return;
    }
  };

  for entry in entries {
    match entry {
      serde_json::Value::String(s) => {
        if s.starts_with('{') || s.starts_with('!') {
          // It's a file pattern (e.g., "{projectRoot}/**/*" or "!{projectRoot}/**/*.spec.ts")
          resolved.push((origin.to_string(), s.clone()));
        } else {
          // It's a reference to another named input (e.g., "sharedGlobals").
          // The referenced name becomes the new origin so leaf patterns are
          // attributed to it, not to the input that referenced it.
          resolve_named_input(s, s, all_inputs, resolved, visited);
        }
      }
      serde_json::Value::Object(_) => {
        // Object entries like {"runtime": "node"} or {"externalDependencies": [...]}
        // These are not file patterns — skip them
        debug!("Skipping object entry in namedInput '{}'", name);
      }
      _ => {
        debug!("Skipping unexpected entry type in namedInput '{}'", name);
      }
    }
  }
}

impl ResolvedNamedInputs {
  /// Check if a changed file matches any global invalidation pattern.
  /// `file_path` should be relative to workspace root. Returns the matching
  /// `GlobalPattern` so callers learn *which* namedInput triggered.
  pub fn matches_global_pattern(&self, file_path: &Path) -> Option<&GlobalPattern> {
    let path_str = file_path.to_str()?;

    let opts = glob::MatchOptions {
      case_sensitive: true,
      require_literal_separator: false,
      require_literal_leading_dot: false,
    };

    for gp in &self.global_patterns {
      if gp.pattern.matches_with(path_str, opts) {
        debug!(
          "File '{}' matches global pattern '{}' from namedInput '{}'",
          path_str,
          gp.pattern.as_str(),
          gp.named_input
        );
        return Some(gp);
      }
    }
    None
  }

  /// Check if a changed file should be excluded by negation patterns.
  /// `file_path` should be relative to workspace root.
  /// `project_root` is the project's root directory (relative to workspace root).
  pub fn is_negated(&self, file_path: &Path, project_root: &Path) -> bool {
    if self.negation_patterns.is_empty() {
      return false;
    }

    // Check if file is under this project root
    let relative = match file_path.strip_prefix(project_root) {
      Ok(rel) => rel,
      Err(_) => return false,
    };

    let relative_str = match relative.to_str() {
      Some(s) => s,
      None => return false,
    };

    let opts = glob::MatchOptions {
      case_sensitive: true,
      require_literal_separator: false,
      require_literal_leading_dot: false,
    };

    for pat in &self.negation_patterns {
      if pat.matches_with(relative_str, opts) {
        debug!(
          "File '{}' excluded by negation pattern '!{{projectRoot}}/{}'",
          file_path.display(),
          pat.as_str()
        );
        return true;
      }
    }
    false
  }

  /// Check if a file is negated by any of the given project roots.
  /// Returns true if the file matches a negation pattern for any project that owns it.
  pub fn is_negated_by_any_project(&self, file_path: &Path, project_roots: &[&Path]) -> bool {
    if self.negation_patterns.is_empty() {
      return false;
    }
    project_roots
      .iter()
      .any(|root| self.is_negated(file_path, root))
  }
}

/// Collect every changed file that triggers global invalidation along with the
/// namedInput it matched. Returns an empty vec when no file matches.
///
/// Note: a previous version of this function returned only the first match,
/// which was enough to short-circuit the affected-projects calculation but
/// hid information from the report. The report now lists every trigger so
/// users can see at a glance *why* a run was globally invalidated.
pub fn check_global_invalidation(
  inputs: &ResolvedNamedInputs,
  changed_files: &[ChangedFile],
) -> Vec<GlobalTrigger> {
  let mut triggers = Vec::new();
  for changed_file in changed_files {
    if let Some(gp) = inputs.matches_global_pattern(&changed_file.file_path) {
      debug!(
        "Global invalidation triggered by {:?} (namedInput: {})",
        changed_file.file_path, gp.named_input
      );
      triggers.push(GlobalTrigger {
        file: changed_file.file_path.clone(),
        named_input: gp.named_input.clone(),
        raw_pattern: gp.raw_pattern.clone(),
      });
    }
  }
  triggers
}

/// Filter out changed files that match negation patterns from namedInputs.
/// Returns a new vector with negated files removed.
pub fn filter_negated_files(
  inputs: &ResolvedNamedInputs,
  changed_files: Vec<ChangedFile>,
  projects: &[Project],
) -> Vec<ChangedFile> {
  if inputs.negation_patterns.is_empty() {
    return changed_files;
  }

  let project_roots: Vec<&Path> = projects.iter().map(|p| p.root.as_path()).collect();

  let before = changed_files.len();
  let filtered: Vec<ChangedFile> = changed_files
    .into_iter()
    .filter(|f| !inputs.is_negated_by_any_project(&f.file_path, &project_roots))
    .collect();
  let after = filtered.len();

  if before != after {
    debug!(
      "Filtered {} files by namedInputs negation patterns ({} → {})",
      before - after,
      before,
      after
    );
  }
  filtered
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn write_nx_json(root: &Path, content: &str) {
    fs::write(root.join("nx.json"), content).unwrap();
  }

  #[test]
  fn test_resolve_global_patterns() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["{projectRoot}/**/*", "sharedGlobals"],
          "sharedGlobals": [
            "{workspaceRoot}/babel.config.json",
            "{workspaceRoot}/patches/*"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 2);
    // Both global patterns should be attributed to the `sharedGlobals`
    // namedInput, not to `default` (which only referenced it).
    for gp in &resolved.global_patterns {
      assert_eq!(gp.named_input, "sharedGlobals");
    }
    assert!(resolved
      .matches_global_pattern(&PathBuf::from("babel.config.json"))
      .is_some());
    assert!(resolved
      .matches_global_pattern(&PathBuf::from("patches/some-patch.patch"))
      .is_some());
    assert!(resolved
      .matches_global_pattern(&PathBuf::from("src/index.ts"))
      .is_none());
  }

  #[test]
  fn test_resolve_negation_patterns() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": [
            "{projectRoot}/**/*",
            "!{projectRoot}/**/*.figma.tsx",
            "!{projectRoot}/**/*.stories.tsx"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.negation_patterns.len(), 2);

    let project_root = PathBuf::from("libs/ui");
    assert!(resolved.is_negated(
      &PathBuf::from("libs/ui/src/Button.figma.tsx"),
      &project_root
    ));
    assert!(resolved.is_negated(
      &PathBuf::from("libs/ui/src/Button.stories.tsx"),
      &project_root
    ));
    assert!(!resolved.is_negated(&PathBuf::from("libs/ui/src/Button.tsx"), &project_root));
  }

  #[test]
  fn test_recursive_resolution() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["{projectRoot}/**/*", "sharedGlobals"],
          "sharedGlobals": ["{workspaceRoot}/babel.config.json", "ciInputs"],
          "ciInputs": ["{workspaceRoot}/ci/utils.Jenkinsfile"]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 2);
    let babel = resolved
      .matches_global_pattern(&PathBuf::from("babel.config.json"))
      .expect("babel.config.json should match");
    assert_eq!(babel.named_input, "sharedGlobals");
    let jenkins = resolved
      .matches_global_pattern(&PathBuf::from("ci/utils.Jenkinsfile"))
      .expect("ci/utils.Jenkinsfile should match");
    // Even though the chain is default → sharedGlobals → ciInputs, the leaf
    // pattern is attributed to its most specific origin (ciInputs).
    assert_eq!(jenkins.named_input, "ciInputs");
  }

  #[test]
  fn test_circular_reference_handled() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["a"],
          "a": ["b"],
          "b": ["a", "{workspaceRoot}/file.json"]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 1);
    assert!(resolved
      .matches_global_pattern(&PathBuf::from("file.json"))
      .is_some());
  }

  #[test]
  fn test_object_entries_skipped() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": [
            "{projectRoot}/**/*",
            {"runtime": "node"},
            "{workspaceRoot}/global.json"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 1);
  }

  #[test]
  fn test_no_named_inputs_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(root, r#"{"npmScope": "myorg"}"#);

    assert!(resolve_from_nx_json(root).is_none());
  }

  #[test]
  fn test_no_default_named_input_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "production": ["{projectRoot}/**/*"]
        }
      }"#,
    );

    // No "default" key → no patterns resolved
    assert!(resolve_from_nx_json(root).is_none());
  }

  #[test]
  fn test_is_negated_by_any_project() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": [
            "{projectRoot}/**/*",
            "!{projectRoot}/**/*.spec.ts"
          ]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();

    let root1 = PathBuf::from("libs/a");
    let root2 = PathBuf::from("libs/b");
    let roots: Vec<&Path> = vec![root1.as_path(), root2.as_path()];

    assert!(resolved.is_negated_by_any_project(&PathBuf::from("libs/a/src/foo.spec.ts"), &roots));
    assert!(!resolved.is_negated_by_any_project(&PathBuf::from("libs/a/src/foo.ts"), &roots));
  }

  #[test]
  fn test_check_global_invalidation_returns_all_matches() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["{projectRoot}/**/*", "sharedGlobals", "ciInputs"],
          "sharedGlobals": [
            "{workspaceRoot}/nx.json",
            "{workspaceRoot}/package.json"
          ],
          "ciInputs": ["{workspaceRoot}/.github/workflows/ci.yml"]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    let changed = vec![
      ChangedFile {
        file_path: PathBuf::from(".github/workflows/ci.yml"),
        changed_lines: vec![],
        deleted_lines: vec![],
      },
      ChangedFile {
        file_path: PathBuf::from("nx.json"),
        changed_lines: vec![],
        deleted_lines: vec![],
      },
      ChangedFile {
        file_path: PathBuf::from("package.json"),
        changed_lines: vec![],
        deleted_lines: vec![],
      },
      ChangedFile {
        file_path: PathBuf::from("libs/foo/src/index.ts"),
        changed_lines: vec![],
        deleted_lines: vec![],
      },
    ];

    let triggers = check_global_invalidation(&resolved, &changed);
    assert_eq!(
      triggers.len(),
      3,
      "all three workspace-root files should be reported as triggers"
    );

    let by_file: HashMap<_, _> = triggers
      .iter()
      .map(|t| (t.file.clone(), t.named_input.clone()))
      .collect();
    assert_eq!(
      by_file.get(&PathBuf::from(".github/workflows/ci.yml")),
      Some(&"ciInputs".to_string())
    );
    assert_eq!(
      by_file.get(&PathBuf::from("nx.json")),
      Some(&"sharedGlobals".to_string())
    );
    assert_eq!(
      by_file.get(&PathBuf::from("package.json")),
      Some(&"sharedGlobals".to_string())
    );
  }

  #[test]
  fn test_check_global_invalidation_empty_when_no_match() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["{projectRoot}/**/*", "sharedGlobals"],
          "sharedGlobals": ["{workspaceRoot}/nx.json"]
        }
      }"#,
    );

    let resolved = resolve_from_nx_json(root).unwrap();
    let changed = vec![ChangedFile {
      file_path: PathBuf::from("libs/foo/src/index.ts"),
      changed_lines: vec![],
      deleted_lines: vec![],
    }];

    assert!(check_global_invalidation(&resolved, &changed).is_empty());
  }

  fn write_turbo_json(root: &Path, content: &str) {
    fs::write(root.join("turbo.json"), content).unwrap();
  }

  #[test]
  fn test_turbo_global_dependencies_resolved() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_turbo_json(
      root,
      r#"{
        "globalDependencies": [".env", "config/*.json"],
        "tasks": { "build": {} }
      }"#,
    );

    let resolved = resolve_from_turbo_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 2);
    // Turborepo has no per-project negation concept.
    assert!(resolved.negation_patterns.is_empty());

    let env = resolved
      .matches_global_pattern(&PathBuf::from(".env"))
      .expect(".env should match");
    assert_eq!(env.named_input, "globalDependencies");
    assert_eq!(env.raw_pattern, ".env");

    assert!(resolved
      .matches_global_pattern(&PathBuf::from("config/app.json"))
      .is_some());
    assert!(resolved
      .matches_global_pattern(&PathBuf::from("packages/ui/src/index.ts"))
      .is_none());
  }

  #[test]
  fn test_turbo_without_global_dependencies_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_turbo_json(root, r#"{"tasks": {"build": {}}}"#);
    assert!(resolve_from_turbo_json(root).is_none());
  }

  #[test]
  fn test_turbo_unusable_patterns_skipped() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    // Negations and root-escaping globs are skipped rather than mis-compiled
    // into positive patterns; the remaining entries still resolve. "..config.json"
    // merely starts with two literal dots — it is a legitimate root file, not a
    // root-escaping "../" path, so it must NOT be skipped.
    write_turbo_json(
      root,
      r#"{
        "globalDependencies": ["!.env.local", "../outside/**", "/abs/path", ".env", "..config.json"]
      }"#,
    );

    let resolved = resolve_from_turbo_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 2);
    assert_eq!(resolved.global_patterns[0].raw_pattern, ".env");
    assert_eq!(resolved.global_patterns[1].raw_pattern, "..config.json");
    assert!(resolved
      .matches_global_pattern(&PathBuf::from(".env.local"))
      .is_none());
    assert!(resolved
      .matches_global_pattern(&PathBuf::from("..config.json"))
      .is_some());
  }

  #[test]
  fn test_turbo_leading_dot_slash_stripped_before_compiling() {
    // A leading "./" compiles into a glob::Pattern that can never match
    // git's relative paths (which never start with "./"), silently making
    // the pattern a no-op. It must be stripped before compiling.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_turbo_json(root, r#"{"globalDependencies": ["./.env"]}"#);

    let resolved = resolve_from_turbo_json(root).unwrap();
    assert_eq!(resolved.global_patterns.len(), 1);
    // Display text keeps the user's original spelling...
    assert_eq!(resolved.global_patterns[0].raw_pattern, "./.env");
    // ...but the compiled pattern matches the path git actually reports.
    assert!(resolved
      .matches_global_pattern(&PathBuf::from(".env"))
      .is_some());
  }

  #[test]
  fn test_turbo_malformed_config_returns_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_turbo_json(root, "{ not json ");
    assert!(resolve_from_turbo_json(root).is_none());
  }

  #[test]
  fn test_resolve_global_inputs_prefers_nx_over_turbo() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_nx_json(
      root,
      r#"{
        "namedInputs": {
          "default": ["{projectRoot}/**/*", "sharedGlobals"],
          "sharedGlobals": ["{workspaceRoot}/.nvmrc"]
        }
      }"#,
    );
    write_turbo_json(root, r#"{"globalDependencies": [".env"]}"#);

    let resolved = resolve_global_inputs(root).expect("nx.json should resolve");
    assert!(resolved
      .matches_global_pattern(&PathBuf::from(".nvmrc"))
      .is_some());
    assert!(
      resolved
        .matches_global_pattern(&PathBuf::from(".env"))
        .is_none(),
      "turbo.json globalDependencies must be ignored in an Nx workspace"
    );
  }

  #[test]
  fn test_resolve_global_inputs_falls_back_to_turbo() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    write_turbo_json(root, r#"{"globalDependencies": [".env"]}"#);

    let resolved = resolve_global_inputs(root).expect("turbo.json should resolve");
    assert!(resolved
      .matches_global_pattern(&PathBuf::from(".env"))
      .is_some());
  }

  #[test]
  fn test_resolve_global_inputs_none_without_config() {
    let dir = tempfile::TempDir::new().unwrap();
    assert!(resolve_global_inputs(dir.path()).is_none());
  }
}
