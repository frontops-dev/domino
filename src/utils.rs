use crate::types::Project;
use std::path::{Path, PathBuf};

/// Extensions considered as source files (analyzed by Oxc parser)
const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx"];

/// Check if a file is a source file (TypeScript/JavaScript)
/// These are files that can be parsed by the Oxc parser
pub fn is_source_file(path: &Path) -> bool {
  path
    .extension()
    .and_then(|ext| ext.to_str())
    .map(|ext| SOURCE_EXTENSIONS.contains(&ext))
    .unwrap_or(false)
}

/// Pre-built index from sourceRoot to project names for O(unique_roots) lookups
/// instead of O(total_projects) on every call.
pub struct ProjectIndex {
  /// Each entry is a unique sourceRoot paired with all project names that share it.
  entries: Vec<(PathBuf, Vec<String>)>,
}

impl ProjectIndex {
  /// Build the index once from a slice of projects.
  pub fn new(projects: &[Project]) -> Self {
    // Group project names by sourceRoot
    let mut map: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for project in projects {
      if let Some(entry) = map
        .iter_mut()
        .find(|(root, _)| *root == project.source_root)
      {
        entry.1.push(project.name.clone());
      } else {
        map.push((project.source_root.clone(), vec![project.name.clone()]));
      }
    }
    Self { entries: map }
  }

  /// Find ALL project names whose sourceRoot is a prefix of `file_path`.
  pub fn get_package_names_by_path(&self, file_path: &Path) -> Vec<String> {
    let mut result = Vec::new();
    for (root, names) in &self.entries {
      if file_path.starts_with(root) {
        result.extend(names.iter().cloned());
      }
    }
    result
  }
}

/// Convert line number to byte offset in source text
/// Line numbers are 1-indexed, returns 0-indexed byte offset
pub fn line_to_offset(source: &str, line: usize) -> Option<usize> {
  if line == 0 {
    return Some(0);
  }

  source
    .lines()
    .take(line - 1) // line is 1-indexed
    .map(|l| l.len() + 1) // +1 for newline character
    .sum::<usize>()
    .into()
}

/// Convert byte offset to line and column
/// Returns (line, column) both 1-indexed
pub fn offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
  let mut line = 1;
  let mut col = 1;
  let mut current_offset = 0;

  for ch in source.chars() {
    if current_offset >= offset {
      break;
    }

    if ch == '\n' {
      line += 1;
      col = 1;
    } else {
      col += 1;
    }

    current_offset += ch.len_utf8();
  }

  (line, col)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_is_source_file() {
    // Source files
    assert!(is_source_file(Path::new("index.ts")));
    assert!(is_source_file(Path::new("component.tsx")));
    assert!(is_source_file(Path::new("utils.js")));
    assert!(is_source_file(Path::new("app.jsx")));
    assert!(is_source_file(Path::new("path/to/file.ts")));

    // Non-source files
    assert!(!is_source_file(Path::new("styles.css")));
    assert!(!is_source_file(Path::new("template.html")));
    assert!(!is_source_file(Path::new("config.json")));
    assert!(!is_source_file(Path::new("image.png")));
    assert!(!is_source_file(Path::new("data.yaml")));
    assert!(!is_source_file(Path::new("no-extension")));
  }

  #[test]
  fn test_line_to_offset() {
    let source = "line1\nline2\nline3\n";
    assert_eq!(line_to_offset(source, 0), Some(0));
    assert_eq!(line_to_offset(source, 1), Some(0));
    assert_eq!(line_to_offset(source, 2), Some(6)); // After "line1\n"
    assert_eq!(line_to_offset(source, 3), Some(12)); // After "line1\nline2\n"
  }

  #[test]
  fn test_offset_to_line_col() {
    let source = "line1\nline2\nline3\n";
    assert_eq!(offset_to_line_col(source, 0), (1, 1));
    assert_eq!(offset_to_line_col(source, 5), (1, 6));
    assert_eq!(offset_to_line_col(source, 6), (2, 1)); // After newline
    assert_eq!(offset_to_line_col(source, 12), (3, 1));
  }

  #[test]
  fn test_project_index() {
    let projects = vec![
      Project {
        name: "core".to_string(),
        source_root: "libs/core/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "nx".to_string(),
        source_root: "libs/nx/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
    ];

    let index = ProjectIndex::new(&projects);

    assert_eq!(
      index.get_package_names_by_path(Path::new("libs/core/src/index.ts")),
      vec!["core".to_string()]
    );
    assert_eq!(
      index.get_package_names_by_path(Path::new("libs/nx/src/cli.ts")),
      vec!["nx".to_string()]
    );
    assert_eq!(
      index.get_package_names_by_path(Path::new("other/file.ts")),
      Vec::<String>::new()
    );
  }

  #[test]
  fn test_project_index_shared_source_root() {
    let projects = vec![
      Project {
        name: "app-desktop".to_string(),
        source_root: "projects/app-desktop/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "app-desktop-mv3".to_string(),
        source_root: "projects/app-desktop/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "other-project".to_string(),
        source_root: "projects/other/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
    ];

    let index = ProjectIndex::new(&projects);

    // File in shared sourceRoot should match both projects
    let mut result = index.get_package_names_by_path(Path::new("projects/app-desktop/src/main.ts"));
    result.sort();
    assert_eq!(result, vec!["app-desktop", "app-desktop-mv3"]);

    // File in unique sourceRoot should match only one project
    let result = index.get_package_names_by_path(Path::new("projects/other/src/index.ts"));
    assert_eq!(result, vec!["other-project"]);

    // File outside all sourceRoots should match nothing
    let result = index.get_package_names_by_path(Path::new("unknown/file.ts"));
    assert!(result.is_empty());
  }
}
