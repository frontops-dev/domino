use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// Lockfile change detection strategy
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LockfileStrategy {
  /// Lockfile changes are ignored entirely
  None,
  /// Mark projects that import affected deps (no reference chain tracing)
  #[default]
  Direct,
  /// Mark importing projects AND trace full reference chains
  Full,
}

impl fmt::Display for LockfileStrategy {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      LockfileStrategy::None => write!(f, "none"),
      LockfileStrategy::Direct => write!(f, "direct"),
      LockfileStrategy::Full => write!(f, "full"),
    }
  }
}

impl FromStr for LockfileStrategy {
  type Err = String;

  fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
    match s.to_lowercase().as_str() {
      "none" => Ok(LockfileStrategy::None),
      "direct" => Ok(LockfileStrategy::Direct),
      "full" => Ok(LockfileStrategy::Full),
      _ => Err(format!(
        "Invalid lockfile strategy '{}'. Expected: none, direct, full",
        s
      )),
    }
  }
}

/// A project in the workspace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
  /// Project name
  pub name: String,
  /// Path to the project root directory (where project.json lives, relative to workspace root)
  pub root: PathBuf,
  /// Path to the project source root (may differ from root, e.g. "libs/my-lib/src")
  pub source_root: PathBuf,
  /// Path to the project's tsconfig.json (optional)
  pub ts_config: Option<PathBuf>,
  /// Implicit dependencies (projects that should be marked affected when this one changes)
  pub implicit_dependencies: Vec<String>,
  /// Available targets (Nx only)
  pub targets: Vec<String>,
}

/// A file with changed lines
#[derive(Debug, Clone)]
pub struct ChangedFile {
  /// Path to the file (relative to workspace root)
  pub file_path: PathBuf,
  /// New-side line numbers that changed (1-indexed).
  /// Empty for binary files (entire file considered changed) and for
  /// deletion-only files (all lines were removed — see `deleted_lines`).
  pub changed_lines: Vec<usize>,
  /// Old-side (base-revision) line numbers removed by pure-deletion hunks
  /// (`@@ -X,Y +Z,0 @@`), 1-indexed. These lines no longer exist in the
  /// working tree, so the symbol they belonged to can only be recovered by
  /// re-parsing the base revision at these lines — which is how dependents of
  /// deleted code are traced. Empty when the change added or modified lines.
  pub deleted_lines: Vec<usize>,
}

/// A reference to a symbol in the code
#[derive(Debug, Clone)]
pub struct Reference {
  /// File where the reference is located
  pub file_path: PathBuf,
  /// Line number (1-indexed)
  pub line: usize,
  /// Column number (0-indexed)
  #[allow(dead_code)]
  pub column: usize,
}

/// A reference to a non-source asset in a source file
#[derive(Debug, Clone)]
pub struct AssetReference {
  /// The source file containing the reference
  pub source_file: PathBuf,
  /// Line number where the reference appears (1-indexed)
  pub line: usize,
  /// Column number of the reference start (0-indexed)
  #[allow(dead_code)]
  pub column: usize,
  /// The matched path string from the source file (useful for debugging)
  #[allow(dead_code)]
  pub matched_path: String,
}

/// Import information
#[derive(Debug, Clone)]
pub struct Import {
  /// The imported symbol name (from the source file)
  pub imported_name: String,
  /// The local name (in the importing file)
  pub local_name: String,
  /// The module specifier (e.g., "./utils" or "lodash")
  pub from_module: String,
  /// The resolved file path (after module resolution)
  #[allow(dead_code)]
  pub resolved_file: Option<PathBuf>,
  /// Whether this is a type-only import
  #[allow(dead_code)]
  pub is_type_only: bool,
  /// Whether this import comes from a dynamic import() expression.
  /// Dynamic imports with string literal specifiers are treated like static
  /// namespace imports — only explicit member access propagates changes.
  pub is_dynamic: bool,
}

/// Export information
#[derive(Debug, Clone)]
pub struct Export {
  /// The exported symbol name
  pub exported_name: String,
  /// The local name (if different from exported name)
  pub local_name: Option<String>,
  /// If this is a re-export, the module it's re-exported from
  pub re_export_from: Option<String>,
}

/// Configuration for the true affected algorithm
#[derive(Debug, Clone)]
pub struct TrueAffectedConfig {
  /// Current working directory
  pub cwd: PathBuf,
  /// Base branch to compare against
  pub base: String,
  /// Head commit to compare (defaults to working tree)
  pub head: Option<String>,
  /// Root tsconfig path
  #[allow(dead_code)]
  pub root_ts_config: Option<PathBuf>,
  /// Projects in the workspace
  pub projects: Vec<Project>,
  /// Additional file patterns to include
  #[allow(dead_code)]
  pub include: Vec<String>,
  /// Paths to ignore
  #[allow(dead_code)]
  pub ignored_paths: Vec<String>,
  /// Lockfile change detection strategy
  pub lockfile_strategy: LockfileStrategy,
}

/// Result of the true affected analysis
#[derive(Debug, Clone, Serialize)]
pub struct AffectedResult {
  /// List of affected project names
  pub affected_projects: Vec<String>,
  /// Detailed report with causality information (optional)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub report: Option<AffectedReport>,
}

/// Detailed report of affected projects with causality information
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectedReport {
  /// Information about each affected project
  pub projects: Vec<AffectedProjectInfo>,
  /// Changed files that triggered Nx `namedInputs` global invalidation.
  /// Empty for non-global runs.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub global_triggers: Vec<GlobalTrigger>,
  /// Aggregate counts for at-a-glance interpretation of the run.
  pub totals: ReportTotals,
  /// domino crate version that produced this report.
  pub version: &'static str,
  /// When the run started (seconds since Unix epoch). Integer chosen over
  /// ISO-8601 to avoid pulling in a new date-time dependency just for this.
  pub run_started_at_unix_secs: i64,
}

/// A changed file that matched a `{workspaceRoot}/...` pattern in nx.json's
/// `namedInputs`, triggering global invalidation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalTrigger {
  /// File that matched (relative to workspace root).
  pub file: PathBuf,
  /// Name of the `namedInputs` entry whose pattern matched, e.g. `sharedGlobals`.
  pub named_input: String,
  /// Original pattern string from nx.json, e.g. `{workspaceRoot}/nx.json`.
  pub raw_pattern: String,
}

/// Aggregate counts surfaced at the top of the HTML report.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportTotals {
  /// Projects affected purely via global invalidation.
  pub globally_invalidated: usize,
  /// Projects affected via real semantic analysis (DirectChange,
  /// ImportedSymbol, AssetChange, LockfileChange, ...).
  pub semantically_affected: usize,
  /// Projects affected via both global and semantic causes.
  pub overlap: usize,
  /// Total number of files changed in the diff (pre-filtering).
  pub changed_files: usize,
}

/// Information about why a project is affected
#[derive(Debug, Clone, Serialize)]
pub struct AffectedProjectInfo {
  /// Project name
  pub name: String,
  /// Reasons why this project is affected
  pub causes: Vec<AffectCause>,
}

/// Reason why a project is affected
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "type")]
pub enum AffectCause {
  /// Direct change to a file in this project
  #[serde(rename = "direct_change")]
  DirectChange {
    /// File that was changed
    file: PathBuf,
    /// Symbol that was changed (if identified)
    symbol: Option<String>,
    /// Line number where the change occurred
    line: usize,
  },
  /// Imported a changed symbol from another project
  #[serde(rename = "imported_symbol")]
  ImportedSymbol {
    /// Source project that was changed
    source_project: String,
    /// The symbol that was imported
    symbol: String,
    /// File where the import occurs
    via_file: PathBuf,
    /// Original file where symbol was changed
    source_file: PathBuf,
  },
  /// Re-exported a changed symbol
  #[serde(rename = "re_exported")]
  #[allow(dead_code)]
  ReExported {
    /// File that re-exports the symbol
    through_file: PathBuf,
    /// The symbol being re-exported
    symbol: String,
    /// Original source file
    source_file: PathBuf,
  },
  /// Implicit dependency on another affected project
  #[serde(rename = "implicit_dependency")]
  ImplicitDependency {
    /// Project this depends on
    depends_on: String,
  },
  /// Asset file changed and is referenced by source code
  #[serde(rename = "asset_change")]
  AssetChange {
    /// The asset file that changed
    asset_file: PathBuf,
    /// Source file that references the asset
    referenced_in: PathBuf,
    /// Line where the reference appears
    line: usize,
  },
  /// Lockfile dependency changed
  #[serde(rename = "lockfile_change")]
  LockfileChange {
    /// The affected dependency name
    dependency: String,
    /// Source file that imports the dependency
    importing_file: PathBuf,
  },
  /// Global invalidation via Nx namedInputs (e.g., sharedGlobals)
  #[serde(rename = "global_invalidation")]
  GlobalInvalidation {
    /// The file that triggered global invalidation
    file: PathBuf,
    /// The `namedInputs` entry whose pattern matched (e.g. `sharedGlobals`).
    /// Surfaced in the report so the user recognizes the term from their nx.json.
    named_input: String,
  },
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_lockfile_strategy_from_str_valid() {
    assert_eq!(
      "none".parse::<LockfileStrategy>().unwrap(),
      LockfileStrategy::None
    );
    assert_eq!(
      "direct".parse::<LockfileStrategy>().unwrap(),
      LockfileStrategy::Direct
    );
    assert_eq!(
      "full".parse::<LockfileStrategy>().unwrap(),
      LockfileStrategy::Full
    );
  }

  #[test]
  fn test_lockfile_strategy_from_str_case_insensitive() {
    assert_eq!(
      "Direct".parse::<LockfileStrategy>().unwrap(),
      LockfileStrategy::Direct
    );
    assert_eq!(
      "FULL".parse::<LockfileStrategy>().unwrap(),
      LockfileStrategy::Full
    );
    assert_eq!(
      "None".parse::<LockfileStrategy>().unwrap(),
      LockfileStrategy::None
    );
  }

  #[test]
  fn test_lockfile_strategy_from_str_invalid() {
    assert!("invalid".parse::<LockfileStrategy>().is_err());
    assert!("".parse::<LockfileStrategy>().is_err());
    assert!("direkt".parse::<LockfileStrategy>().is_err());
  }

  #[test]
  fn test_lockfile_strategy_display_roundtrip() {
    for strategy in [
      LockfileStrategy::None,
      LockfileStrategy::Direct,
      LockfileStrategy::Full,
    ] {
      let s = strategy.to_string();
      let parsed: LockfileStrategy = s.parse().unwrap();
      assert_eq!(parsed, strategy);
    }
  }

  #[test]
  fn test_lockfile_strategy_default() {
    assert_eq!(LockfileStrategy::default(), LockfileStrategy::Direct);
  }
}
