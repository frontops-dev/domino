# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

domino is a high-performance Rust implementation of **True Affected** - semantic change detection for monorepos. It's a drop-in replacement for the TypeScript version of traf, using the Oxc parser for 3-5x faster performance. The tool analyzes actual code changes at the AST level (not just file changes) and follows symbol references across the entire workspace to determine which projects are truly affected by changes.

This is a dual-purpose project:
- A standalone CLI binary (`domino`) built with Cargo
- An npm package with N-API bindings for Node.js integration

## Build and Development Commands

### Rust Binary Development

```bash
# Build debug binary
cargo build

# Build release binary (optimized)
cargo build --release

# Run from source
cargo run -- affected --all

# Run unit tests
cargo test

# Run integration tests (MUST be serial due to git state)
cargo test --test integration_test -- --test-threads=1

# Format code
cargo fmt

# Lint code
cargo clippy

# Enable debug logging
RUST_LOG=domino=debug cargo run -- affected
```

### Node.js Package Development

```bash
# Build N-API bindings (release)
yarn build

# Build N-API bindings (debug)
yarn build:debug

# Run JavaScript tests (using ava)
yarn test

# Format all code (Rust + JS + TOML)
yarn format

# Lint JavaScript/TypeScript
yarn lint
```

### Running Tests

**Important**: Integration tests modify git state and MUST run serially:
```bash
cargo test --test integration_test -- --test-threads=1
```

Unit tests can run in parallel:
```bash
cargo test --lib
```

When changing code, always check for related tests and adjust them accordingly.

## CLI Usage

```bash
# Show all projects
domino affected --all

# Find affected projects (vs origin/main)
domino affected

# Use different base branch
domino affected --base origin/develop

# JSON output
domino affected --json

# Debug logging
domino affected --debug

# Set working directory
domino affected --cwd /path/to/monorepo
```

## Architecture

### Core Algorithm Flow

The true-affected detection follows this pipeline (see `src/core.rs`):

1. **Git Diff Analysis** → Parse git diffs to identify changed files and specific changed lines
2. **Semantic Parsing** → Parse all TypeScript/JavaScript files using Oxc to build AST and semantic model
3. **Symbol Resolution** → Identify which symbols (functions, classes, constants, etc.) were actually modified based on changed line ranges
4. **Reference Finding** → Recursively find all cross-file references to those symbols using the import/export graph
5. **Project Mapping** → Map affected files back to their owning projects in the workspace

### Key Components

- **`src/core.rs`**: Main algorithm orchestration - implements the 5-step pipeline above
- **`src/git.rs`**: Git integration - parses diffs to identify changed files and line ranges
- **`src/semantic/analyzer.rs`**: Workspace-wide semantic analysis using Oxc
  - Parses all files and builds AST
  - Tracks imports/exports for each file
  - Builds reverse import index: `(source_file, symbol_name) -> [(importing_file, local_name)]`
- **`src/semantic/reference_finder.rs`**: Cross-file reference tracking
  - Uses `oxc_resolver` for module resolution (same as Rolldown/Nova)
  - Maintains resolution cache for performance
  - Recursively follows import chains to find all affected files
- **`src/workspace/`**: Project discovery for different monorepo tools
  - `nx.rs`: Nx workspace support (nx.json, project.json)
  - `turbo.rs`: Turborepo support (turbo.json)
  - `workspaces.rs`: Generic npm/yarn/pnpm/bun workspaces
- **`src/cli.rs`**: CLI interface using clap
- **`src/lib.rs`**: N-API bindings for Node.js integration
- **`src/profiler.rs`**: Performance profiling utilities
- **`src/report.rs`**: Detailed analysis reports showing why projects are affected

### Critical Data Structures

**Import Index** (`WorkspaceAnalyzer::import_index`):
- Maps `(source_file, symbol_name)` to all locations that import it
- Key for efficient reverse lookup when finding references
- Example: `(utils.ts, "formatDate")` → `[(app.ts, "formatDate", "./utils"), (helper.ts, "format", "./utils")]`

**Resolution Cache** (`ReferenceFinder::resolution_cache`):
- Caches module resolution results: `(from_file, specifier)` → `resolved_path`
- Uses `RefCell` for interior mutability (not thread-safe currently)
- Critical for performance when following import chains

### Module Resolution

Uses `oxc_resolver` with TypeScript-aware configuration:
- Looks for `tsconfig.base.json` in workspace root for path mappings
- Supports extensions: `.ts`, `.tsx`, `.js`, `.jsx`, `.d.ts`
- Handles both relative imports and workspace path aliases

### Testing Strategy

- **Unit tests** (`cargo test --lib`): Test individual components in isolation
- **Integration tests** (`tests/integration_test.rs`): End-to-end tests with real git repos
  - Uses `tempfile` for isolated test directories
  - Creates git repos programmatically
  - Must run serially (`--test-threads=1`) due to git state
- **CLI tests** (`tests/cli_test.rs`): Test CLI interface using `assert_cmd`
- **JavaScript tests** (`__test__/index.spec.ts`): Test N-API bindings using ava

## Important Technical Details

### Oxc Integration

This project is built on the Oxc parser ecosystem:
- `oxc_parser`: Fast JavaScript/TypeScript parsing
- `oxc_semantic`: Semantic analysis and symbol table
- `oxc_resolver`: Module resolution (same engine as Rolldown and Nova)
- `oxc_allocator`: Arena allocator for AST nodes (lifetime management)

### Lifetime Management

The `WorkspaceAnalyzer` uses `'static` lifetimes for Oxc semantic data via memory transmutation. This is safe because:
- Allocators are stored alongside their semantic data in `FileSemanticData`
- Data is never accessed after its allocator is dropped
- All access is contained within the analyzer's lifetime

### Performance Considerations

- Import index enables O(1) reverse lookup instead of scanning all files
- Resolution cache prevents redundant module resolution
- Oxc parser is 3-5x faster than TypeScript compiler
- Release builds use aggressive optimizations: `lto=true`, `codegen-units=1`

### N-API Bindings

The crate is configured as both `cdylib` (for Node.js) and `rlib` (for Rust):
```toml
[lib]
crate-type = ["cdylib", "rlib"]
```

This allows:
- Building native Node.js modules with `napi-rs`
- Running Rust unit tests that import the library code

## Workspace Types Supported

1. **Nx**: Detects via `nx.json`, reads project configuration from `project.json` files
2. **Turborepo**: Detects via `turbo.json`, reads workspace configuration from root `package.json`
3. **Generic workspaces**: Falls back to npm/yarn/pnpm/bun workspace detection from `package.json`
