use thiserror::Error;

#[derive(Error, Debug)]
pub enum DominoError {
  #[error("IO error: {0}")]
  Io(#[from] std::io::Error),

  #[error("Regex error: {0}")]
  Regex(#[from] regex::Error),

  #[error("Parse error: {0}")]
  Parse(String),

  #[error("Module resolution error: {0}")]
  #[allow(dead_code)]
  ModuleResolution(String),

  #[error("Project not found: {0}")]
  #[allow(dead_code)]
  ProjectNotFound(String),

  #[error("File not found: {0}")]
  FileNotFound(String),

  #[error("Invalid configuration: {0}")]
  #[allow(dead_code)]
  InvalidConfig(String),

  #[error("Semantic analysis error: {0}")]
  #[allow(dead_code)]
  Semantic(String),

  #[error("{0}")]
  Other(String),
}

pub type Result<T> = std::result::Result<T, DominoError>;
