# Review Branch

Review all changes on the current feature branch compared to a base branch, using specialist agents, then run pre-commit checks.

Usage: `/review-branch [base-branch]`

Default base branch: `main`

---

## Step 1: Verify Branch State

Determine the base branch from `$ARGUMENTS`, defaulting to `main` if not provided.

```bash
git branch --show-current
```

- If on the base branch: inform user "You're on {base}. Switch to a feature branch first." and stop
- Check divergence from base:
  ```bash
  git log {base}..HEAD --oneline
  ```
- If no commits beyond base: inform user "Branch has no commits beyond {base}." and stop

## Step 2: Get Base SHA

```bash
git merge-base {base} HEAD
```

Store as `{baseSha}`.

## Step 3: Get Changed Files

```bash
git diff --name-status {baseSha}...HEAD
```

Also get stats:
```bash
git diff --stat {baseSha}...HEAD
```

## Step 4: Show Branch Summary

```
Branch: {current_branch}
Commits: N ahead of main
Changed files: M (+additions, -deletions)

Files:
  A  src/new_file.rs
  M  src/core.rs
  M  src/git.rs
  D  src/old_file.rs
```

## Step 5: Spawn Analysis Agent

Use the `analysis-agent` (from `.claude/agents/analysis-agent.md`) to determine which specialists are needed.

Provide:
- The baseSha
- The full list of changed files with their status (added, modified, deleted)
- The diff summary

The analysis agent returns recommendations on which specialists to invoke.

## Step 6: Spawn Specialists in Parallel

Based on the analysis agent's recommendations, spawn the appropriate specialists from `.claude/agents/`:

| Specialist | When to Invoke |
|---|---|
| `rust-specialist` | Any `.rs` or `Cargo.toml` changes |
| `security-specialist` | Files with `unsafe`, FFI boundaries, new dependencies |
| `test-specialist` | Test files changed, or source files changed without corresponding test changes |
| `ai-specialist` | Changes in `.claude/` directory |

Each specialist receives:
- Their assigned files from the diff
- The baseSha for comparison

## Step 7: Validate and Present

Follow the full validation and presentation process from `.claude/shared/review-workflow.md`:

1. Check CLAUDE.md guidelines for documented intentional patterns
2. Spot-check code by reading files and verifying claims
3. Assess project applicability
4. Deduplicate findings across specialists
5. Classify as Relevant, Needs User Input, or Not Relevant

Present the structured report using the severity-grouped format from the shared workflow.

## Step 8: Run Pre-Commit Checks

After presenting the review, run the quality checks:

```bash
cargo fmt --all -- --check 2>&1
```

```bash
cargo clippy --all-targets --all-features -- -D warnings 2>&1
```

```bash
cargo test --lib 2>&1
```

```bash
cargo test --test integration_test -- --test-threads=1 2>&1
```

Present results alongside the review:
```
Quality checks:
  [PASS]  cargo fmt -- Passed
  [FAIL]  cargo clippy -- 2 warnings
  [PASS]  Unit tests -- 45 passed
  [PASS]  Integration tests -- 12 passed
```

## Step 9: Post-Review Options

Offer the user these options:

1. **Fix issues** -- Automatically fix reported critical and medium issues
2. **Create PR** -- Push the branch and create a pull request (follows the `/pr-create` workflow)
3. **Show filtered issues** -- Display issues that were filtered out and why
4. **Do nothing** -- Skip and continue working

If user chooses to fix issues:
- Apply each fix using the Edit tool
- Run `cargo clippy` and `cargo fmt` after Rust file changes
- Present a summary of changes made
- Offer to commit the fixes
