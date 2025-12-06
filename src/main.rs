mod cli;
mod core;
mod error;
mod git;
mod profiler;
mod semantic;
mod types;
mod utils;
mod workspace;

fn main() {
  if let Err(e) = cli::run() {
    eprintln!("Error: {}", e);
    std::process::exit(1);
  }
}
