mod cli;
mod core;
mod error;
mod git;
mod lockfile;
mod profiler;
mod report;
mod semantic;
mod tsconfig;
mod types;
mod utils;
mod workspace;

fn main() {
  if let Err(e) = cli::run() {
    eprintln!("Error: {}", e);
    std::process::exit(1);
  }
}
