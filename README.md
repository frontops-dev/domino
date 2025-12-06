# domino

A high-performance Rust implementation of **True Affected** - semantic change detection for monorepos using the Oxc parser.

## Overview

domino is a drop-in replacement for the TypeScript version of [traf](https://github.com/@lemonade-hq/traf), providing the same semantic analysis capabilities with significantly better performance thanks to Rust and the Oxc parser.

## Features

- **Semantic Change Detection**: Analyzes actual code changes at the AST level, not just file changes
- **Cross-File Reference Tracking**: Follows symbol references across your entire workspace
- **Fast Oxc Parser**: 3-5x faster than TypeScript's compiler API
- **Nx & Turbo Support**: Works with both Nx and Turborepo monorepos
- **Module Resolution**: Uses oxc_resolver (same as Rolldown and Nova) for accurate module resolution

## Quick Start

```bash
# Clone and build
git clone git@github.com:jsverse/domino.git
cd domino
cargo build --release

# Run in your monorepo
cd /path/to/your/monorepo
/path/to/domino/target/release/domino affected
```

## Installation

### From Source

```bash
cd domino
cargo build --release
```

The binary will be available at `./target/release/domino`.

## Usage

### Basic Commands

```bash
# Show all projects in the workspace
domino affected --all

# Find affected projects (compared to origin/main)
domino affected

# Use a different base branch
domino affected --base origin/develop

# Output as JSON
domino affected --json

# Enable debug logging
domino affected --debug
```

### Options

- `--base <BRANCH>`: Base branch to compare against (default: `origin/main`)
- `--all`: Show all projects regardless of changes
- `--json`: Output results as JSON
- `--debug`: Enable debug logging
- `--cwd <PATH>`: Set the current working directory

## How It Works

1. **Git Diff Analysis**: Detects which files and specific lines have changed
2. **Semantic Parsing**: Parses all TypeScript/JavaScript files using Oxc
3. **Symbol Resolution**: Identifies which symbols (functions, classes, constants) were modified
4. **Reference Finding**: Recursively finds all cross-file references to those symbols
5. **Project Mapping**: Maps affected files to their owning projects

## Performance

Thanks to Rust and Oxc, domino is significantly faster than the TypeScript version:

- **Parsing**: 3-5x faster using Oxc
- **Memory**: Lower memory footprint
- **Startup**: Near-instant startup time

## Comparison with TypeScript Version

| Feature      | TypeScript                     | Rust                     |
| ------------ | ------------------------------ | ------------------------ |
| Parser       | ts-morph (TypeScript compiler) | Oxc parser               |
| Speed        | Baseline                       | 3-5x faster              |
| Memory       | Baseline                       | ~50% less                |
| Binary Size  | Requires Node.js + deps        | Single standalone binary |
| Startup Time | ~1-2s                          | <100ms                   |

## Architecture

### Core Components

- **Git Integration** (`src/git.rs`): Parses git diffs to identify changed files and lines
- **Workspace Discovery** (`src/workspace/`): Discovers projects in Nx and Turbo workspaces
- **Semantic Analyzer** (`src/semantic/analyzer.rs`): Uses Oxc to parse and analyze TypeScript/JavaScript
- **Reference Finder** (`src/semantic/reference_finder.rs`): Tracks cross-file symbol references
- **Core Algorithm** (`src/core.rs`): Orchestrates the affected detection logic

### Key Technologies

- **[Oxc](https://github.com/oxc-project/oxc)**: High-performance JavaScript/TypeScript parser and toolchain
- **oxc_resolver**: Module resolution (used by Rolldown, Nova, knip)
- **clap**: CLI argument parsing
- **git2**: Git integration
- **serde**: JSON/YAML parsing

## Development

### Quick Reference

```bash
# Build (debug)
cargo build

# Build (release)
cargo build --release

# Run tests
cargo test

# Run integration tests (must be serial)
cargo test --test integration_test -- --test-threads=1

# Run from source
cargo run -- affected --all

# Format code
cargo fmt

# Lint code
cargo clippy

# Enable debug logging
RUST_LOG=domino=debug cargo run -- affected
```

## License

Same as the original traf project.

## Credits

This is a Rust port of the original [traf](https://github.com/@lemonade-hq/traf) TypeScript implementation.

Built with:

- [Oxc](https://github.com/oxc-project/oxc) - The JavaScript Oxidation Compiler
- [oxc_resolver](https://github.com/oxc-project/oxc-resolver) - Fast module resolution
