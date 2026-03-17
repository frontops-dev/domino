use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// Lockfile change detection strategy
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
  /// Path to the project source root
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
  /// Line numbers that changed (1-indexed).
  /// Empty for binary files (entire file considered changed).
  pub changed_lines: Vec<usize>,
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
  /// Whether this import comes from a dynamic import() expression
  /// Dynamic imports get conservative treatment for namespace resolution
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
pub struct AffectedReport {
  /// Information about each affected project
  pub projects: Vec<AffectedProjectInfo>,
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
