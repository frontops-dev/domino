pub mod nx;
pub mod rush;
pub mod turbo;
pub mod workspaces;

use crate::error::{DominoError, Result};
use crate::types::Project;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use std::path::Path;

const STATIC_EXCLUDES: &[&str] = &["!node_modules/", "!dist/", "!__fixtures__/"];

/// Build a directory walker that respects .gitignore, custom ignore files, and exclusion patterns.
pub(crate) fn build_walker(
  cwd: &Path,
  extra_excludes: &[&str],
  custom_ignore_files: &[&str],
) -> Result<ignore::Walk> {
  let mut overrides = OverrideBuilder::new(cwd);
  for pattern in STATIC_EXCLUDES.iter().chain(extra_excludes.iter()) {
    overrides
      .add(pattern)
      .map_err(|e| DominoError::Other(format!("Override error: {}", e)))?;
  }
  let overrides = overrides
    .build()
    .map_err(|e| DominoError::Other(format!("Override build error: {}", e)))?;

  let mut builder = WalkBuilder::new(cwd);
  builder
    .hidden(false)
    .git_ignore(true)
    .git_global(true)
    .git_exclude(true)
    .overrides(overrides);

  for filename in custom_ignore_files {
    builder.add_custom_ignore_filename(filename);
  }

  Ok(builder.build())
}

/// Detect workspace type and discover projects
pub fn discover_projects(cwd: &Path) -> Result<Vec<Project>> {
  // Try Nx first
  if nx::is_nx_workspace(cwd) {
    return nx::get_projects(cwd);
  }

  // Try Turbo (turbo.json)
  if turbo::is_turbo_workspace(cwd) {
    return turbo::get_projects(cwd);
  }

  // Try generic workspaces (npm/yarn/pnpm/bun)
  if workspaces::is_workspace(cwd) {
    return workspaces::get_projects(cwd);
  }

  // Try Rush (rush.json) — checked last to avoid interfering with existing workspace types
  if rush::is_rush_workspace(cwd) {
    return rush::get_projects(cwd);
  }

  // If none found, return empty
  Ok(vec![])
}
