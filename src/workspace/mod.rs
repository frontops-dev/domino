pub mod nx;
pub mod turbo;

use crate::error::Result;
use crate::types::Project;
use std::path::Path;

/// Detect workspace type and discover projects
pub fn discover_projects(cwd: &Path) -> Result<Vec<Project>> {
  // Try Nx first
  if nx::is_nx_workspace(cwd) {
    return nx::get_projects(cwd);
  }

  // Try Turbo
  if turbo::is_turbo_workspace(cwd) {
    return turbo::get_projects(cwd);
  }

  // If neither, return empty
  Ok(vec![])
}
