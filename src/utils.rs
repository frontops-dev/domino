use crate::tsconfig::TsconfigExcludes;
use crate::types::Project;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use tracing::debug;

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

/// Pre-built index from sourceRoot (and project root) to project names for
/// O(unique_roots) lookups instead of O(total_projects) on every call.
///
/// Also holds per-project tsconfig exclude patterns so that files excluded
/// by a project's tsconfig (e.g. `*.stories.tsx`, `*.spec.ts`) don't count
/// toward marking that project as affected.
pub struct ProjectIndex {
  /// Each entry is a unique sourceRoot paired with all project names that share it.
  entries: Vec<(PathBuf, Vec<String>)>,
  /// Each entry is a unique project root paired with all project names that share it.
  /// Used as a fallback when a file is inside a project's root but outside its sourceRoot
  /// (e.g. config files like project.json, jest.config.js, tsconfig.json).
  root_entries: Vec<(PathBuf, Vec<String>)>,
  /// Compiled exclude patterns per project name.
  excludes: FxHashMap<String, TsconfigExcludes>,
}

impl ProjectIndex {
  /// Build the index from a slice of projects, parsing each project's tsconfig
  /// to extract exclude patterns.
  pub fn new(projects: &[Project], cwd: &Path) -> Self {
    let mut map: Vec<(PathBuf, Vec<String>)> = Vec::new();
    let mut root_map: Vec<(PathBuf, Vec<String>)> = Vec::new();
    let mut excludes = FxHashMap::default();

    for project in projects {
      // Index by sourceRoot (primary)
      if let Some(entry) = map
        .iter_mut()
        .find(|(root, _)| *root == project.source_root)
      {
        entry.1.push(project.name.clone());
      } else {
        map.push((project.source_root.clone(), vec![project.name.clone()]));
      }

      // Index by root (fallback) — only when root differs from sourceRoot
      if project.root != project.source_root {
        if let Some(entry) = root_map.iter_mut().find(|(root, _)| *root == project.root) {
          entry.1.push(project.name.clone());
        } else {
          root_map.push((project.root.clone(), vec![project.name.clone()]));
        }
      }

      if let Some(ts_config) = &project.ts_config {
        if let Some(parsed) = TsconfigExcludes::parse(ts_config, cwd) {
          debug!(
            "Loaded {} exclude patterns for project '{}' from {}",
            parsed.pattern_count(),
            project.name,
            ts_config.display()
          );
          excludes.insert(project.name.clone(), parsed);
        }
      }
    }

    Self {
      entries: map,
      root_entries: root_map,
      excludes,
    }
  }

  /// Find ALL project names whose sourceRoot (or root) is a prefix of `file_path`,
  /// excluding projects whose tsconfig excludes the file.
  ///
  /// Checks sourceRoot entries first (with tsconfig exclude filtering), then falls
  /// back to root entries for files that live inside a project's root but outside its
  /// sourceRoot (e.g. config files like project.json, jest.config.js).
  pub fn get_package_names_by_path(&self, file_path: &Path) -> Vec<String> {
    let mut result = Vec::new();
    let mut matched_source_root = false;
    // Primary: match against sourceRoot (with tsconfig exclude filtering)
    for (root, names) in &self.entries {
      if file_path.starts_with(root) {
        matched_source_root = true;
        for name in names {
          if let Some(excl) = self.excludes.get(name) {
            if excl.is_excluded(file_path) {
              debug!(
                "File {:?} excluded by tsconfig for project '{}'",
                file_path, name
              );
              continue;
            }
          }
          result.push(name.clone());
        }
      }
    }
    // Fallback: match against project root for files outside sourceRoot
    // Only when no sourceRoot matched at all (not when excluded by tsconfig).
    // tsconfig excludes are not applied here — config files should always count.
    if !matched_source_root {
      for (root, names) in &self.root_entries {
        if file_path.starts_with(root) {
          for name in names {
            result.push(name.clone());
          }
        }
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
    let tmp = tempfile::TempDir::new().unwrap();
    let projects = vec![
      Project {
        name: "core".to_string(),
        root: "libs/core/src".into(),
        source_root: "libs/core/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "nx".to_string(),
        root: "libs/nx/src".into(),
        source_root: "libs/nx/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
    ];

    let index = ProjectIndex::new(&projects, tmp.path());

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
    let tmp = tempfile::TempDir::new().unwrap();
    let projects = vec![
      Project {
        name: "app-desktop".to_string(),
        root: "projects/app-desktop/src".into(),
        source_root: "projects/app-desktop/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "app-desktop-mv3".to_string(),
        root: "projects/app-desktop/src".into(),
        source_root: "projects/app-desktop/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "other-project".to_string(),
        root: "projects/other/src".into(),
        source_root: "projects/other/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
    ];

    let index = ProjectIndex::new(&projects, tmp.path());

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

  #[test]
  fn test_project_index_tsconfig_excludes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path();

    let lib_dir = cwd.join("libs/ui-widgets");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(
      lib_dir.join("tsconfig.lib.json"),
      r#"{ "exclude": ["**/*.spec.ts", "**/*.stories.tsx"] }"#,
    )
    .unwrap();

    let projects = vec![Project {
      name: "ui-widgets".to_string(),
      root: "libs/ui-widgets".into(),
      source_root: "libs/ui-widgets/src".into(),
      ts_config: Some(lib_dir.join("tsconfig.lib.json")),
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    let index = ProjectIndex::new(&projects, cwd);

    assert_eq!(
      index.get_package_names_by_path(Path::new("libs/ui-widgets/src/index.ts")),
      vec!["ui-widgets"],
      "normal source files should match"
    );
    assert!(
      index
        .get_package_names_by_path(Path::new("libs/ui-widgets/src/Grid.stories.tsx"))
        .is_empty(),
      "stories files should be excluded"
    );
    assert!(
      index
        .get_package_names_by_path(Path::new("libs/ui-widgets/src/utils.spec.ts"))
        .is_empty(),
      "spec files should be excluded"
    );
  }

  #[test]
  fn test_project_index_root_fallback() {
    let tmp = tempfile::TempDir::new().unwrap();
    let projects = vec![
      Project {
        name: "my-app".to_string(),
        root: "apps/my-app".into(),
        source_root: "apps/my-app/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      Project {
        name: "my-lib".to_string(),
        root: "libs/my-lib".into(),
        source_root: "libs/my-lib/src".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
      // Project where root == sourceRoot (no fallback needed)
      Project {
        name: "simple".to_string(),
        root: "libs/simple".into(),
        source_root: "libs/simple".into(),
        ts_config: None,
        implicit_dependencies: vec![],
        targets: vec![],
      },
    ];

    let index = ProjectIndex::new(&projects, tmp.path());

    // Source files inside sourceRoot should match (existing behavior)
    assert_eq!(
      index.get_package_names_by_path(Path::new("apps/my-app/src/main.ts")),
      vec!["my-app"]
    );

    // Config files inside root but outside sourceRoot should match via fallback
    assert_eq!(
      index.get_package_names_by_path(Path::new("apps/my-app/project.json")),
      vec!["my-app"],
      "project.json inside root but outside sourceRoot should match"
    );
    assert_eq!(
      index.get_package_names_by_path(Path::new("apps/my-app/jest.config.js")),
      vec!["my-app"],
      "jest.config.js inside root but outside sourceRoot should match"
    );
    assert_eq!(
      index.get_package_names_by_path(Path::new("libs/my-lib/tsconfig.json")),
      vec!["my-lib"],
      "tsconfig.json inside root but outside sourceRoot should match"
    );

    // Files completely outside all roots should still not match
    assert!(index
      .get_package_names_by_path(Path::new("unknown/file.ts"))
      .is_empty());

    // Project where root == sourceRoot should still work normally
    assert_eq!(
      index.get_package_names_by_path(Path::new("libs/simple/index.ts")),
      vec!["simple"]
    );
  }

  #[test]
  fn test_project_index_root_fallback_with_tsconfig_excludes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path();

    let lib_dir = cwd.join("libs/ui-widgets");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(
      lib_dir.join("tsconfig.lib.json"),
      r#"{ "exclude": ["**/*.spec.ts"] }"#,
    )
    .unwrap();

    let projects = vec![Project {
      name: "ui-widgets".to_string(),
      root: "libs/ui-widgets".into(),
      source_root: "libs/ui-widgets/src".into(),
      ts_config: Some(lib_dir.join("tsconfig.lib.json")),
      implicit_dependencies: vec![],
      targets: vec![],
    }];

    let index = ProjectIndex::new(&projects, cwd);

    // Source file in sourceRoot: normal behavior
    assert_eq!(
      index.get_package_names_by_path(Path::new("libs/ui-widgets/src/index.ts")),
      vec!["ui-widgets"]
    );

    // Spec file in sourceRoot should be excluded by tsconfig
    assert!(
      index
        .get_package_names_by_path(Path::new("libs/ui-widgets/src/utils.spec.ts"))
        .is_empty(),
      "spec files in sourceRoot should be excluded"
    );

    // Config file in root (outside sourceRoot) should match via fallback
    // (tsconfig excludes do NOT apply to root fallback)
    assert_eq!(
      index.get_package_names_by_path(Path::new("libs/ui-widgets/jest.config.js")),
      vec!["ui-widgets"],
      "config files in root should match even with tsconfig excludes"
    );
  }
}
