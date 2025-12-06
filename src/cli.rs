use crate::core;
use crate::error::Result;
use crate::profiler::Profiler;
use crate::types::TrueAffectedConfig;
use crate::workspace;
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

#[derive(Parser)]
#[command(name = "domino")]
#[command(about = "True Affected - Semantic change detection for monorepos", long_about = None)]
#[command(version)]
struct Cli {
  #[command(subcommand)]
  command: Commands,

  /// Enable debug logging
  #[arg(short, long, global = true)]
  debug: bool,

  /// CI mode: suppress all logs, only output results
  #[arg(long, global = true)]
  ci: bool,
}

#[derive(Subcommand)]
enum Commands {
  /// Find affected projects
  Affected {
    /// Base branch to compare against
    #[arg(short, long, default_value = "origin/main")]
    base: String,

    /// Current working directory
    #[arg(long)]
    cwd: Option<PathBuf>,

    /// Output as JSON
    #[arg(long)]
    json: bool,

    /// Show all projects regardless of changes
    #[arg(long)]
    all: bool,

    /// Path to root tsconfig
    #[arg(long)]
    ts_config: Option<PathBuf>,

    /// Enable performance profiling (also: DOMINO_PROFILE=1)
    #[arg(long)]
    profile: bool,
  },
}

pub fn run() -> Result<()> {
  let cli = Cli::parse();

  // Setup logging with cleaner formatting
  let log_level = if cli.ci {
    "error" // CI mode: only errors
  } else if cli.debug {
    "debug" // Debug mode: show debug and warnings
  } else {
    "warn" // Default: show warnings
  };
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!("domino={}", log_level).into()),
    )
    .without_time() // Remove timestamps
    .with_target(false) // Remove module names
    .init();

  match cli.command {
    Commands::Affected {
      base,
      cwd,
      json,
      all,
      ts_config,
      profile,
    } => {
      let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap());

      // Enable profiling via --profile flag or DOMINO_PROFILE env var
      let enable_profiling = profile || std::env::var("DOMINO_PROFILE").is_ok();
      if enable_profiling {
        eprintln!("📊 Performance profiling enabled");
      }

      // Auto-detect default branch if using the default value
      let base = if base == "origin/main" {
        crate::git::detect_default_branch(&cwd)
      } else {
        base
      };

      debug!("Discovering projects in {:?}", cwd);

      // Discover projects
      let projects = workspace::discover_projects(&cwd)?;

      if projects.is_empty() {
        eprintln!("{}", "No projects found in workspace".red());
        return Ok(());
      }

      debug!("Found {} projects", projects.len());

      if all {
        // Show all projects
        let all_projects: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();

        if json {
          println!("{}", serde_json::to_string(&all_projects).unwrap());
        } else {
          println!("{}", "All projects:".bold().blue());
          for project in &all_projects {
            println!("  {} {}", "•".blue(), project);
          }
          println!("\n{} {} projects", "Total:".bold(), all_projects.len());
        }

        return Ok(());
      }

      // Create profiler
      let profiler = Arc::new(Profiler::new(enable_profiling));

      // Run true-affected analysis
      let config = TrueAffectedConfig {
        cwd: cwd.clone(),
        base,
        root_ts_config: ts_config,
        projects,
        include: vec![],
        ignored_paths: vec![
          "node_modules".to_string(),
          "dist".to_string(),
          "build".to_string(),
          ".git".to_string(),
        ],
      };

      let result = core::find_affected(config, profiler)?;

      if json {
        println!(
          "{}",
          serde_json::to_string(&result.affected_projects).unwrap()
        );
      } else if result.affected_projects.is_empty() {
        println!("{}", "No affected projects".yellow());
      } else {
        println!("{}", "Affected projects:".bold().green());
        for project in &result.affected_projects {
          println!("  {} {}", "•".green(), project);
        }
        println!(
          "\n{} {} affected project{}",
          "Total:".bold(),
          result.affected_projects.len(),
          if result.affected_projects.len() == 1 {
            ""
          } else {
            "s"
          }
        );
      }

      Ok(())
    }
  }
}
