use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
  /// Line numbers that changed (1-indexed)
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
}

/// Result of the true affected analysis
#[derive(Debug, Clone, Serialize)]
pub struct AffectedResult {
  /// List of affected project names
  pub affected_projects: Vec<String>,
}
