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
# (--no-default-features skips the napi-bindings feature: N-API symbols only
# resolve inside a Node process, so test binaries can't link with it)
cargo test --lib --no-default-features

# Run integration tests (MUST be serial due to git state)
cargo test --no-default-features --test integration_test -- --test-threads=1

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

**Important**: Integration tests modify git state and MUST run serially. Test
binaries must be built with `--no-default-features` (the napi-bindings feature
references N-API symbols that only resolve inside a Node process, so test
executables fail to link with it):

```bash
cargo test --no-default-features --test integration_test -- --test-threads=1
```

Unit tests can run in parallel:

```bash
cargo test --lib --no-default-features
```

The `tests/fixtures/monorepo` fixture is generated from scratch by
`tests/common/mod.rs` on first use (it is gitignored — it contains its own
`.git`, so committing it would turn it into a gitlink). Delete the directory
to force a clean regeneration.

When changing code, always check for related tests and adjust them accordingly.

### Reproducing CI Failures Locally

**Default to a local Linux container, not CI.** Pushing diagnostic commits to
read CI output costs minutes per iteration and pollutes PR history. A container
reproduces Linux-only and timing-dependent failures in seconds and gives an
iteration loop you can run hundreds of times.

```bash
colima start --cpu 6 --memory 10   # once per machine
unset DOCKER_HOST                  # DOCKER_HOST overrides the colima context

docker --context colima run --rm \
  -v "$PWD":/src -w /src \
  -v domino-cargo-reg:/usr/local/cargo/registry \
  -v domino-target-linux:/tmp/t -e CARGO_TARGET_DIR=/tmp/t \
  rust:1.95-bookworm \
  bash -c 'cargo test --lib --no-default-features'
```

Use the image tag matching `rust-toolchain.toml`. The named volumes cache the
crate registry and Linux artifacts, so only the first run is slow.

`Cargo.lock` is gitignored, so CI resolves dependencies fresh — rule version drift
in or out early by comparing `cargo tree` against the CI log's "Locking" lines.

To read one job's log while the run is still in progress (`gh run view --log-failed`
refuses): `gh api repos/<owner>/<repo>/actions/jobs/<job-id>/logs`

Three gotchas, each producing failures that look unrelated to your change:

- **Git worktrees**: a worktree's `.git` is a *file* pointing at an absolute host
  path that does not exist in the container, so every fixture-based test fails
  with `fatal: not a git repository`. Copy the tree and give it a real repo:
  `cp -a /src/. /work/ && cd /work && rm -f .git && git init -q . && git add -A && git commit -qm base`
- **`tests/cli_test.rs` ignores `CARGO_TARGET_DIR`** — it resolves the binary at
  `CARGO_MANIFEST_DIR/target/...`. If a host `target/` is visible in the
  container it execs the macOS binary and fails with `Exec format error`. Remove
  `target/` and let Cargo build in-tree for CLI tests.
- Set `git config --global user.email`/`user.name` in the container. Tests that
  build their own repos configure themselves, but ad-hoc `git commit` will not.

For a flaky failure, loop the prebuilt test binary instead of `cargo test`:

```bash
BIN=$(ls -t /tmp/t/debug/deps/integration_test-* | grep -v '\.d$' | head -1)
for i in $(seq 1 200); do
  $BIN some_test_name --test-threads=1 >/dev/null 2>&1 || echo "fail on run $i"
done
```

**Instrument only inside the failing branch.** Memory-safety and timing bugs are
often Heisenbugs: adding output before the failure point shifts stack layout or
timing and makes them vanish. Put diagnostics in the error path only, and
confirm a fix with the loop, never a single green run.

Reach for CI only when the failure genuinely needs the real runner environment —
cross-compiled targets (musl, Windows), N-API artifact packaging, the npm
publish flow, or a failure that survives a clean local Linux reproduction.

## CLI Usage

```bash
# Show all projects
domino affected --all

# Find affected projects (vs origin/main)
domino affected

# Use different base branch
domino affected --base origin/develop

# Compare a specific head commit (defaults to the working tree)
domino affected --head <sha>

# JSON output
domino affected --json

# Debug logging
domino affected --debug

# CI mode: suppress all logs, output results only
domino affected --ci

# Explicit root tsconfig
domino affected --ts-config tsconfig.base.json

# Performance profiling (equivalent to DOMINO_PROFILE=1)
domino affected --profile

# Generate an HTML dependency graph report
domino affected --report ./affected-report.html

# Lockfile change detection strategy: none | direct | full (default: direct)
domino affected --lockfile-strategy full

# Set working directory
domino affected --cwd /path/to/monorepo
```

Three things the flag list does not show:

- `--base` defaults to `origin/main` only as a clap placeholder. When it is left at that
  default, `git::detect_default_branch` overrides it with the repository's actual default
  branch — so the literal string is not what gets compared.
- `include` and `ignored_paths` exist in `TrueAffectedConfig` and are exposed to Node as
  `include` / `ignoredPaths` (see `index.d.ts`), but have **no CLI flags**: `cli.rs` hardcodes
  `include: vec![]` and a fixed ignore list (`node_modules`, `dist`, `build`, `.git`).
- `--ts-config` is parsed into `TrueAffectedConfig::root_ts_config` and then **never read** —
  nothing in `core.rs` consumes it, so passing it silently changes nothing. Resolution always
  uses `<cwd>/tsconfig.base.json` (`resolve_options.rs`). Same for the `rootTsConfig` /
  `include` / `ignoredPaths` options on the Node API.

## Architecture

### Core Algorithm Flow

The true-affected detection follows this pipeline (see `src/core.rs`):

1. **Git Diff Analysis** → Parse git diffs to identify changed files and specific changed lines
2. **Named Inputs** → Apply Nx `namedInputs` global-invalidation patterns (e.g. `sharedGlobals`): a matching workspace-root file invalidates every project regardless of the semantic result
3. **Semantic Parsing** → Parse all TypeScript/JavaScript files using Oxc to build AST and semantic model
4. **Symbol Resolution** → Identify which symbols (functions, classes, constants, etc.) were actually modified based on changed line ranges
5. **Reference Finding** → Recursively find all cross-file references to those symbols using the import/export graph
6. **Asset References** → Changed _non-source_ files (HTML templates, stylesheets, JSON, images) are matched back to the source files that reference them, which then re-enter reference finding
7. **Lockfile Changes** → Diff the lockfile to find changed direct dependencies, expand them transitively, and mark the source files importing them
8. **Implicit Dependencies** → Pull in projects declared as implicit dependents of an affected project
9. **Project Mapping & Union** → Map affected files back to their owning projects, and union the semantic result with the global-invalidation result from step 2

The inline `// Step N` comments in `core.rs` are **not** a reliable ordering guide — they
reflect the order features were added, not execution order (`Step 6` appears twice,
`Step 5c` runs after `Step 6b`, and there is no `Step 7`).

### Key Components

- **`src/core.rs`**: Main algorithm orchestration - implements the pipeline above
- **`src/git.rs`**: Git integration - parses diffs to identify changed files and line ranges
- **`src/semantic/analyzer.rs`**: Workspace-wide semantic analysis using Oxc
  - Parses all files and builds AST
  - Tracks imports/exports for each file
  - Builds reverse import index: `(source_file, symbol_name) -> [(importing_file, local_name)]`
- **`src/semantic/reference_finder.rs`**: Cross-file reference tracking
  - Uses `oxc_resolver` for module resolution (same as Rolldown/Nova)
  - Maintains resolution cache for performance
  - Recursively follows import chains to find all affected files
- **`src/semantic/resolve_options.rs`**: Shared `oxc_resolver` configuration used by **both**
  resolution paths (import-index builder and reference finder) - deliberately centralised to
  prevent the two from drifting. Also home of `is_workspace_specifier` (see pitfall below)
- **`src/semantic/assets.rs`**: Finds source-file references to non-source assets, so a changed
  template or stylesheet can be traced to the code that uses it
- **`src/lockfile.rs`**: Lockfile diffing for npm/yarn/pnpm/bun - detects changed direct
  dependencies, builds a reverse dependency graph for transitive impact, then maps results to the
  source files importing them. Refuses lockfiles over 256 MB to avoid OOM on constrained CI runners
- **`src/named_inputs.rs`**: Parses Nx `namedInputs` into compiled global-invalidation and negation
  glob patterns. Each pattern retains the `namedInput` name it came from so reports can echo the
  term the user actually wrote in `nx.json`
- **`src/tsconfig.rs`**: tsconfig loading - follows `extends` chains (depth-capped) and strips
  comments before parsing
- **`src/utils.rs`**: Source-file predicate, plus the pre-built sourceRoot→project index that makes
  project lookup O(unique_roots) rather than O(projects) per call, and per-project tsconfig
  `exclude` patterns (so an excluded `*.spec.ts` does not mark its project affected)
- **`src/workspace/`**: Project discovery for different monorepo tools
  - `nx.rs`: Nx workspace support (nx.json, project.json)
  - `turbo.rs`: Turborepo detection only - a 17-line shim that checks for `turbo.json` then
    delegates to `workspaces.rs`; `turbo.json` itself is never parsed
  - `rush.rs`: Rush support (`rush.json` `projects` array)
  - `workspaces.rs`: Generic npm/yarn/pnpm/bun workspaces
- **`src/cli.rs`**: CLI interface using clap
- **`src/lib.rs`**: N-API bindings for Node.js integration
- **`src/types.rs`**: Shared config and result types (`TrueAffectedConfig`, `Project`, `ChangedFile`, `LockfileStrategy`)
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

### Workspace Specifier Matching (Known Pitfall)

**IMPORTANT**: In many Nx monorepos the project name (from `project.json`) does NOT match
the npm package name or the tsconfig path alias used in import statements. For example:

| Nx project name | tsconfig path alias / import specifier |
|---|---|
| `ui-widgets` | `@acme/shared-ui-widgets` |
| `my-lib` | `@acme/my-lib` |

The `is_workspace_specifier` function (in `resolve_options.rs`) is a performance guard
that short-circuits the resolver for external packages. It MUST check **both** project
names **and** tsconfig path alias keys. If it only checks project names, imports using
tsconfig aliases will be silently classified as external and dropped from the import
index — completely breaking cross-project affected detection.

Any performance optimization that filters import specifiers must account for this
name mismatch. Always add an integration test covering the "project name != import
specifier" scenario when touching resolution or specifier filtering logic.

### Testing Strategy

- **Unit tests** (`cargo test --lib`): Test individual components in isolation
- **Integration tests** (`tests/integration_test.rs`): End-to-end tests with real git repos
  - Uses `tempfile` for isolated test directories
  - Creates git repos programmatically
  - Must run serially (`--test-threads=1`) due to git state
- **CLI tests** (`tests/cli_test.rs`): Test CLI interface using `assert_cmd`
- **JavaScript tests** (`__test__/index.spec.ts`): Test N-API bindings using ava
- Prefer a per-test `TempDir` repo for new tests (`scaffold_lib_app_repo`, `TempNxRepo`) over the shared `tests/fixtures/monorepo` + `TestBranch` fixture — that shared mutable fixture is why `--test-threads=1` is required
- To assert on or dump analyzer state: `WorkspaceAnalyzer::new(projects, &cwd, Arc::new(Profiler::new(false)))`; its `files`/`imports`/`exports`/`import_index` fields are `pub`

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

**Every parse helper that returns `FileSemanticData` must arena-allocate the AST
root** — `let program = &*allocator.alloc(parse_result.program);` before
`SemanticBuilder::build(program)`. `Parser::parse` returns `Program` *by value on
the stack*, and the builder records it as the root `AstKind::Program` node, so
building from `&parse_result.program` leaves the root dangling into a dead frame
once the parsing function returns. The symptom is subtle and intermittent: the
Program node reads back a garbage span (typically `0..0`), which still contains
offset 0 with span size 0, so it beats the real declaration in
`find_top_level_symbols`' smallest-enclosing-node search and symbols on line 1
silently stop resolving — dependents are then never marked affected.

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
2. **Turborepo**: Detects via `turbo.json`, then delegates entirely to generic workspace
   discovery — `turbo.json` contents are never read, so Turbo-specific config (pipelines,
   `globalDependencies`) has no effect on detection
3. **Rush**: Detects via `rush.json`, reads its `projects` array
4. **Generic workspaces**: Falls back to npm/yarn/pnpm/bun workspace detection from `package.json`

## Repo Mechanics

- The toolchain is pinned to 1.95.0 in `rust-toolchain.toml`; `Cargo.toml` declares a lower
  `rust-version` (1.89.0) as the supported MSRV. Use the pinned version for local containers.
- **`Cargo.lock` is gitignored**, so CI resolves dependencies fresh on every run. Dependency
  version drift can therefore appear in CI with no local change to reproduce it.
- `.husky/pre-commit` already enforces `cargo fmt --all -- --check` and
  `cargo clippy --all-targets --all-features -- -D warnings`, so steps 2 and 3 of the checklist
  below are automatic. **Tests are not run by the hook.** The same hook runs `lint-staged`, which
  applies `prettier --write` to staged `js/ts/tsx/yml/yaml/md/json` and `taplo format` to `.toml` —
  so editing this file and committing it will also normalise its existing Markdown formatting.
- Version bumps go through `yarn version` → `napi version` + `scripts/sync-cargo-version.js`,
  which keeps `Cargo.toml` in sync with `package.json`.
- Preview releases are published by `scripts/publish-preview.js`, covered by
  `tests/publish-preview.test.js` (run in the CI `lint` job, not by `cargo test`).

## Pre-Commit Checklist

Before committing changes, opening PRs, or pushing code, **ALWAYS** complete the following steps:

### 1. Clean Up External References

- **Remove any references to external repositories** used during debugging
- Obfuscate repository names, project names, and file paths from examples
- Use generic names like "app-client", "Component.tsx", "getHelperValue()" instead of real names
- Check commit messages, PR descriptions, code comments, and test data

### 2. Run Linting

```bash
cargo clippy --all-targets --all-features
```

Fix any warnings or errors before committing.

### 3. Run Formatting

```bash
cargo fmt --all
```

Ensure all code follows Rust formatting standards.

### 4. Run Tests

```bash
# Run unit tests
cargo test --lib --no-default-features

# Run integration tests (must be serial due to git state)
cargo test --no-default-features --test integration_test -- --test-threads=1
cargo test --no-default-features --test cli_test -- --test-threads=1

# Run doc-tests. ALWAYS pass --no-default-features. With `napi-bindings` on, a
# doctest binary cannot link the host-provided `napi_*` symbols, and rustdoc
# counts that link failure as the expected compile failure. Any `compile_fail`
# doctest (e.g. the FileSemanticData soundness guard) therefore still reports
# "ok" even when the snippet it is supposed to reject compiles again — i.e. the
# regression it exists to catch is silently masked.
cargo test --doc --no-default-features

# For JavaScript/Node.js bindings
yarn test

# Publish-preview script (run by the CI lint job, not by cargo test)
# NOTE: this rewrites the tracked files under tests/fixtures/test-repo/ with
# machine-local absolute paths — `git checkout -- tests/fixtures/test-repo/`
# afterwards so they are not committed.
node --test tests/publish-preview.test.js
```

All tests must pass before committing.

### 5. Rust Code Quality Review

Use the `@agent-rust-specialist` to review:

- Memory safety and ownership patterns
- Error handling and Result usage
- Performance considerations
- API design and documentation
- Rust best practices and idioms

## PR Requirements

When creating pull requests:

1. **Title**: Clear, descriptive, follows conventional commits format (`fix:`, `feat:`, etc.)

2. **Description must include**:
   - Problem statement
   - Solution approach
   - Key changes made
   - Testing performed
   - Related issues (use `#issue_number` format)
   - Breaking changes (if any)

3. **Code must**:
   - Pass all automated checks (lint, format, tests)
   - Be reviewed by rust-specialist agent
   - Include tests for new functionality
   - Update documentation if APIs changed

4. **Obfuscation**:
   - No external repository names in code or docs
   - No customer/client project names
   - No real file paths or internal structure from external projects
   - Use generic, illustrative examples
