use crate::error::Result;
use crate::types::Project;
use std::path::Path;

use super::workspaces;

/// Check if the current directory is a Turbo workspace
/// (Turbo-specific detection via turbo.json)
pub fn is_turbo_workspace(cwd: &Path) -> bool {
  cwd.join("turbo.json").exists()
}

/// Get all Turbo projects in the workspace
/// Delegates to the generic workspaces module for actual project discovery
pub fn get_projects(cwd: &Path) -> Result<Vec<Project>> {
  workspaces::get_projects(cwd)
}
