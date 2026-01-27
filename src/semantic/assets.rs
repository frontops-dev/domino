//! Asset reference finder for detecting non-source file dependencies
//!
//! This module provides functionality to find source files that reference
//! non-source assets (HTML templates, CSS stylesheets, JSON configs, images, etc.)
//!
//! The algorithm matches traf's approach:
//! 1. Extract changed non-source files from git diff
//! 2. Search source files for quoted strings containing the asset filename
//! 3. Resolve paths to verify the import actually points to the changed file
//! 4. Return source file references for further analysis

use crate::error::Result;
use crate::types::AssetReference;
use crate::utils::is_source_file;
use ignore::WalkBuilder;
use regex::Regex;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Finds references to non-source assets in source files
pub struct AssetReferenceFinder {
  /// Workspace root directory
  cwd: PathBuf,
  /// Cache for compiled regex patterns: filename -> regex
  regex_cache: RefCell<rustc_hash::FxHashMap<String, Regex>>,
}

impl AssetReferenceFinder {
  /// Create a new asset reference finder
  pub fn new(cwd: &Path) -> Self {
    Self {
      cwd: cwd.to_path_buf(),
      regex_cache: RefCell::new(rustc_hash::FxHashMap::default()),
    }
  }

  /// Find all source files that reference the given asset file
  ///
  /// # Arguments
  /// * `asset_path` - Path to the asset file (relative to workspace root)
  ///
  /// # Returns
  /// A vector of `AssetReference` containing source files and line numbers
  /// where the asset is referenced
  pub fn find_references(&self, asset_path: &Path) -> Result<Vec<AssetReference>> {
    let file_name = match asset_path.file_name().and_then(|n| n.to_str()) {
      Some(name) => name,
      None => {
        debug!("Asset path has no filename: {:?}", asset_path);
        return Ok(vec![]);
      }
    };

    debug!("Searching for references to asset: {}", file_name);

    // Build regex pattern: ['"`](?P<path>[^'"`]*{escaped_filename})['"`]
    let pattern = self.get_or_create_pattern(file_name)?;

    let mut references = Vec::new();

    // Walk source files using ignore crate (respects .gitignore)
    for entry in WalkBuilder::new(&self.cwd)
      .hidden(false) // Include hidden files
      .git_ignore(true) // Respect .gitignore
      .git_exclude(true) // Respect .git/info/exclude
      .build()
      .filter_map(|e| e.ok())
    {
      let path = entry.path();

      // Skip directories and non-source files
      if path.is_dir() || !is_source_file(path) {
        continue;
      }

      // Search this file for asset references
      if let Some(file_refs) = self.search_file(path, &pattern, asset_path)? {
        references.extend(file_refs);
      }
    }

    debug!(
      "Found {} references to asset '{}'",
      references.len(),
      file_name
    );
    Ok(references)
  }

  /// Search a single source file for references to the asset
  fn search_file(
    &self,
    source_file: &Path,
    pattern: &Regex,
    asset_path: &Path,
  ) -> Result<Option<Vec<AssetReference>>> {
    // Quick check: does the file contain the asset filename at all?
    let file_name = asset_path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("");

    let content = match fs::read_to_string(source_file) {
      Ok(c) => c,
      Err(_) => return Ok(None), // Skip files we can't read
    };

    // Early exit if filename not found
    if !content.contains(file_name) {
      return Ok(None);
    }

    let mut references = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
      for captures in pattern.captures_iter(line) {
        if let Some(path_match) = captures.name("path") {
          let rel_path = path_match.as_str();

          // Resolve the path and verify it points to the asset
          if self.path_resolves_to(source_file, rel_path, asset_path) {
            references.push(AssetReference {
              source_file: source_file
                .strip_prefix(&self.cwd)
                .unwrap_or(source_file)
                .to_path_buf(),
              line: line_num + 1, // 1-indexed
              column: path_match.start(),
              matched_path: rel_path.to_string(),
            });

            debug!(
              "Found reference to '{}' in {:?} at line {}",
              rel_path,
              source_file,
              line_num + 1
            );
          }
        }
      }
    }

    if references.is_empty() {
      Ok(None)
    } else {
      Ok(Some(references))
    }
  }

  /// Check if a relative path in a source file resolves to the asset path
  fn path_resolves_to(&self, source_file: &Path, rel_path: &str, asset_path: &Path) -> bool {
    // Get the directory containing the source file
    let source_dir = source_file.parent().unwrap_or(Path::new("."));

    // Resolve the relative path from the source file's directory
    let resolved = if rel_path.starts_with("./") || rel_path.starts_with("../") {
      self.cwd.join(source_dir).join(rel_path)
    } else {
      // Absolute or bare path - just join with source dir
      self.cwd.join(source_dir).join(rel_path)
    };

    // Normalize both paths for comparison
    let resolved_normalized = self.normalize_path(&resolved);
    let asset_normalized = self.normalize_path(&self.cwd.join(asset_path));

    resolved_normalized == asset_normalized
  }

  /// Normalize a path for comparison (resolve . and ..)
  fn normalize_path(&self, path: &Path) -> PathBuf {
    let mut components = Vec::new();

    for component in path.components() {
      match component {
        std::path::Component::ParentDir => {
          components.pop();
        }
        std::path::Component::CurDir => {}
        _ => {
          components.push(component);
        }
      }
    }

    components.iter().collect()
  }

  /// Get or create a compiled regex pattern for the given filename
  fn get_or_create_pattern(&self, file_name: &str) -> Result<Regex> {
    let mut cache = self.regex_cache.borrow_mut();

    if let Some(pattern) = cache.get(file_name) {
      return Ok(pattern.clone());
    }

    // Escape special regex characters in the filename
    let escaped = regex::escape(file_name);

    // Pattern: ['"`](?P<path>[^'"`]*{escaped_filename})['"`]
    // This matches:
    // - templateUrl: './hero.component.html' (Angular)
    // - import logo from "./logo.png" (ES6)
    // - require('../config.json') (CommonJS)
    // - url(`./bg.png`) (CSS-in-JS)
    let pattern_str = format!(r#"['"`](?P<path>[^'"`]*{})['\"`]"#, escaped);

    let pattern = Regex::new(&pattern_str)?;
    cache.insert(file_name.to_string(), pattern.clone());

    Ok(pattern)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  fn create_test_workspace() -> TempDir {
    let temp = TempDir::new().expect("Failed to create temp dir");
    fs::create_dir_all(temp.path().join("src/components")).expect("Failed to create dirs");
    temp
  }

  #[test]
  fn test_find_references_single_quote() {
    let temp = create_test_workspace();
    let cwd = temp.path();

    // Create asset file
    fs::write(cwd.join("src/components/hero.html"), "<h1>Hero</h1>").unwrap();

    // Create source file with single-quoted reference
    fs::write(
      cwd.join("src/components/hero.component.ts"),
      r#"@Component({
  templateUrl: './hero.html',
})
export class HeroComponent {}
"#,
    )
    .unwrap();

    let finder = AssetReferenceFinder::new(cwd);
    let refs = finder
      .find_references(Path::new("src/components/hero.html"))
      .unwrap();

    assert_eq!(refs.len(), 1);
    assert_eq!(
      refs[0].source_file,
      PathBuf::from("src/components/hero.component.ts")
    );
    assert_eq!(refs[0].line, 2);
    assert_eq!(refs[0].matched_path, "./hero.html");
  }

  #[test]
  fn test_find_references_double_quote() {
    let temp = create_test_workspace();
    let cwd = temp.path();

    // Create files
    fs::write(cwd.join("src/components/styles.css"), ".btn {}").unwrap();
    fs::write(
      cwd.join("src/components/button.ts"),
      r#"import "./styles.css";
export function Button() {}
"#,
    )
    .unwrap();

    let finder = AssetReferenceFinder::new(cwd);
    let refs = finder
      .find_references(Path::new("src/components/styles.css"))
      .unwrap();

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].line, 1);
    assert_eq!(refs[0].matched_path, "./styles.css");
  }

  #[test]
  fn test_find_references_backtick() {
    let temp = create_test_workspace();
    let cwd = temp.path();

    fs::write(cwd.join("src/components/config.json"), "{}").unwrap();
    fs::write(
      cwd.join("src/components/app.ts"),
      "const template = `./config.json`;",
    )
    .unwrap();

    let finder = AssetReferenceFinder::new(cwd);
    let refs = finder
      .find_references(Path::new("src/components/config.json"))
      .unwrap();

    assert_eq!(refs.len(), 1);
  }

  #[test]
  fn test_path_resolution_parent_dir() {
    let temp = create_test_workspace();
    let cwd = temp.path();

    // Create asset in parent directory
    fs::create_dir_all(cwd.join("src/assets")).unwrap();
    fs::write(cwd.join("src/assets/logo.png"), "png-data").unwrap();

    // Create source file that references via ../
    fs::write(
      cwd.join("src/components/header.ts"),
      r#"const logo = require('../assets/logo.png');
"#,
    )
    .unwrap();

    let finder = AssetReferenceFinder::new(cwd);
    let refs = finder
      .find_references(Path::new("src/assets/logo.png"))
      .unwrap();

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].matched_path, "../assets/logo.png");
  }

  #[test]
  fn test_no_match_different_file() {
    let temp = create_test_workspace();
    let cwd = temp.path();

    // Create two files with similar names
    fs::write(cwd.join("src/components/styles.css"), ".btn {}").unwrap();
    fs::write(cwd.join("src/components/other-styles.css"), ".other {}").unwrap();

    // Reference only one of them
    fs::write(
      cwd.join("src/components/button.ts"),
      r#"import "./styles.css";
"#,
    )
    .unwrap();

    let finder = AssetReferenceFinder::new(cwd);

    // Should NOT find references to other-styles.css
    let refs = finder
      .find_references(Path::new("src/components/other-styles.css"))
      .unwrap();
    assert!(refs.is_empty());
  }

  #[test]
  fn test_multiple_references_same_file() {
    let temp = create_test_workspace();
    let cwd = temp.path();

    fs::write(cwd.join("src/components/theme.css"), ".theme {}").unwrap();
    fs::write(
      cwd.join("src/components/app.ts"),
      r#"import './theme.css';
const themePath = './theme.css';
require('./theme.css');
"#,
    )
    .unwrap();

    let finder = AssetReferenceFinder::new(cwd);
    let refs = finder
      .find_references(Path::new("src/components/theme.css"))
      .unwrap();

    assert_eq!(refs.len(), 3);
  }

  #[test]
  fn test_angular_component_decorator() {
    let temp = create_test_workspace();
    let cwd = temp.path();

    fs::write(
      cwd.join("src/components/hero.component.html"),
      "<h1>Hero</h1>",
    )
    .unwrap();
    fs::write(cwd.join("src/components/hero.component.css"), ".hero {}").unwrap();

    fs::write(
      cwd.join("src/components/hero.component.ts"),
      r#"import { Component } from '@angular/core';

@Component({
  selector: 'app-hero',
  templateUrl: './hero.component.html',
  styleUrls: ['./hero.component.css'],
})
export class HeroComponent {
  title = 'Hero';
}
"#,
    )
    .unwrap();

    let finder = AssetReferenceFinder::new(cwd);

    // Find HTML template references
    let html_refs = finder
      .find_references(Path::new("src/components/hero.component.html"))
      .unwrap();
    assert_eq!(html_refs.len(), 1);
    assert_eq!(html_refs[0].line, 5);

    // Find CSS references
    let css_refs = finder
      .find_references(Path::new("src/components/hero.component.css"))
      .unwrap();
    assert_eq!(css_refs.len(), 1);
    assert_eq!(css_refs[0].line, 6);
  }
}
