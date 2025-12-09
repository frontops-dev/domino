pub mod nx;
pub mod turbo;
pub mod workspaces;

use crate::error::Result;
use crate::types::Project;
use std::path::Path;

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

  // If none found, return empty
  Ok(vec![])
}
