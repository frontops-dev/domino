# Shared Agent Guidelines

Comprehensive guidelines for all specialist agents working on the domino project.

## Git Operations

### Base Commit Reference

Always use `baseSha` (the PR's base commit SHA), NOT branch names. Using branch names can include unrelated merged commits and produce incorrect diffs.

**Correct:**

```bash
# Get changed files (memory efficient, no full content)
git diff --name-status ${baseSha}...HEAD

# Get diff for a specific file
git diff ${baseSha}...HEAD -- path/to/file.rs

# View file at base commit (pre-change version)
git show ${baseSha}:path/to/file.rs

# View file at HEAD (current version)
git show HEAD:path/to/file.rs
```

**Incorrect:**

```bash
# DO NOT use branch names — includes merged commits
git diff origin/main...HEAD

# DO NOT fetch full diff upfront — causes memory issues on large PRs
git diff ${baseSha}...HEAD
```

### Memory-Efficient Diff Strategy

1. First, get the file list: `git diff --name-status ${baseSha}...HEAD`
2. Then, diff individual files as needed: `git diff ${baseSha}...HEAD -- <file>`
3. Never fetch the full diff of all files at once

---

## Diff-Aware Review Rules

Only report issues on code that is relevant to the current change.

### What to Report

- **Changed/added lines** — Lines directly in the diff. Full review applies.
- **Unchanged code directly impacted by changes** — For example, a function whose caller changed, or code that depends on a modified type. Mark with `"i": true` in output.
- **Critical issues in adjacent unchanged code** — Only for critical severity (memory safety, soundness). Mark with `"i": true`.

### What NOT to Report

- Pre-existing issues unrelated to the change
- Style issues in unchanged code
- Refactoring suggestions for untouched modules

### File Status Handling

| Status | Meaning | Review Approach |
|--------|---------|-----------------|
| `A` (added) | New file | Review all lines freely |
| `M` (modified) | Changed file | Run `git diff ${baseSha}...HEAD -- <file>`, focus on changed lines + impact |
| `D` (deleted) | Removed file | Check if deletion breaks imports/references in other changed files |
| `R` (renamed) | Renamed/moved file | Use `git diff -M ${baseSha}...HEAD -- <file>` to see only content changes beyond the rename |

---

## Verification Rules

These rules are mandatory. Violations produce unreliable reviews.

1. **NEVER report issues without reading files with the Read tool first.** Do not rely solely on diff output.
2. **Quote exact code snippets** from the file. Verify line numbers match the current HEAD version, not the diff hunk headers.
3. **Check if the issue is pre-existing** by running `git show ${baseSha}:path/to/file.rs` and comparing. If the same issue existed before this PR, do not report it.
4. **If you discover during writing that something is NOT an issue, DELETE it entirely.** Do not include self-contradicting findings like "This looks like a problem, but actually it's fine." Remove the finding completely.

---

## Rust-Specific File Context

| File Pattern | Context | How to Handle |
|---|---|---|
| `src/**/*.rs` | Production source code | Full review |
| `tests/*.rs` | Integration tests | Allow `unwrap()`, test-specific patterns |
| `tests/fixtures/` | Test fixture data | Do not flag for code quality |
| `__test__/*.spec.ts` | JS/N-API binding tests | Different rules than Rust code |
| `target/` | Build output | Skip entirely |
| `node_modules/` | JS dependencies | Skip entirely |
| `*.lock` | Lock files (Cargo.lock, yarn.lock) | Skip unless security concern |
| `benches/` | Benchmarks | Allow performance-specific patterns |

---

## Standard Patterns to Accept (Do Not Flag)

The following patterns are intentional in this codebase. Do not report them as issues:

- **`unsafe` blocks with documented `// SAFETY:` comments** — These have been reviewed and justified.
- **`unwrap()` / `expect()` in test code** (`#[cfg(test)]` or `tests/` directory) — Standard test practice.
- **`#[allow(clippy::...)]` annotations** — Assume intentional suppression with good reason.
- **`transmute` in the semantic analyzer** — Documented lifetime management pattern for Oxc integration. See `src/semantic/analyzer.rs`.
- **Generated code or macro output** — Do not review generated artifacts for style.
- **`cfg(test)` modules** — Test-only code follows relaxed rules.

---

## Output Rules

- **Never write temporary files.** Include all content directly in your response.
- **Follow the JSONL format** specified in `review-output-format.md` for code review findings.
- **Use confidence scores** from `confidence-scoring.md` for every finding.
- **Include the SUMMARY line** as the first line of output.
