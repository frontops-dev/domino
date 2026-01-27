#![deny(clippy::all)]

pub mod cli;
pub mod core;
pub mod error;
pub mod git;
pub mod profiler;
pub mod report;
pub mod semantic;
pub mod types;
pub mod utils;
pub mod workspace;

pub use error::{DominoError, Result};
pub use profiler::Profiler;
pub use types::*;

// N-API bindings (only compiled when napi-bindings feature is enabled)
#[cfg(feature = "napi-bindings")]
mod napi_bindings {
  use super::*;
  use napi::bindgen_prelude::*;
  use napi_derive::napi;
  use std::path::PathBuf;
  use std::sync::Arc;

  #[napi(object)]
  pub struct NapiProject {
    pub name: String,
    pub source_root: String,
    pub ts_config: Option<String>,
    pub implicit_dependencies: Vec<String>,
    pub targets: Vec<String>,
  }

  impl From<Project> for NapiProject {
    fn from(project: Project) -> Self {
      Self {
        name: project.name,
        source_root: project.source_root.to_string_lossy().to_string(),
        ts_config: project.ts_config.map(|p| p.to_string_lossy().to_string()),
        implicit_dependencies: project.implicit_dependencies,
        targets: project.targets,
      }
    }
  }

  impl From<NapiProject> for Project {
    fn from(project: NapiProject) -> Self {
      Self {
        name: project.name,
        source_root: PathBuf::from(project.source_root),
        ts_config: project.ts_config.map(PathBuf::from),
        implicit_dependencies: project.implicit_dependencies,
        targets: project.targets,
      }
    }
  }

  #[napi(object)]
  pub struct FindAffectedOptions {
    pub cwd: String,
    pub base: String,
    pub root_ts_config: Option<String>,
    pub projects: Vec<NapiProject>,
    pub include: Option<Vec<String>>,
    pub ignored_paths: Option<Vec<String>>,
    pub enable_profiling: Option<bool>,
  }

  #[napi(object)]
  pub struct AffectedResultResponse {
    pub affected_projects: Vec<String>,
  }

  /// Find affected projects in a monorepo
  #[napi]
  pub fn find_affected(options: FindAffectedOptions) -> napi::Result<AffectedResultResponse> {
    let cwd = PathBuf::from(&options.cwd);
    let projects: Vec<Project> = options.projects.into_iter().map(Into::into).collect();

    let profiler = Arc::new(Profiler::new(options.enable_profiling.unwrap_or(false)));

    let config = TrueAffectedConfig {
      cwd,
      base: options.base,
      root_ts_config: options.root_ts_config.map(PathBuf::from),
      projects,
      include: options.include.unwrap_or_default(),
      ignored_paths: options.ignored_paths.unwrap_or_default(),
    };

    let result =
      core::find_affected(config, profiler).map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(AffectedResultResponse {
      affected_projects: result.affected_projects,
    })
  }

  /// Discover projects in a workspace (Nx or Turborepo)
  #[napi]
  pub fn discover_projects(cwd: String) -> napi::Result<Vec<NapiProject>> {
    let cwd_path = PathBuf::from(cwd);
    let projects =
      workspace::discover_projects(&cwd_path).map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(projects.into_iter().map(Into::into).collect())
  }
}

#[cfg(feature = "napi-bindings")]
pub use napi_bindings::*;
