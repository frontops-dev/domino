# Pre-Commit Checks

Run all quality checks from the project's pre-commit checklist before committing.

Usage: `/pre-commit`

---

## Step 1: External Reference Check

Scan all staged files for potential leaks of external repository names, project names, or real file paths that should be obfuscated per the CLAUDE.md guidelines.

```bash
git diff --cached --name-only
```

For each staged file, read its content and check for:
- References to external repositories or organization names 
- Customer or client project names
- Real file paths from external projects
- Internal structure details from other codebases

If any are found, list them with file and line number and warn the user before proceeding.

Result: `PASS` (clean) or `WARN` (references found, list them)

## Step 2: Format Check

```bash
cargo fmt --all -- --check 2>&1
```

- If exit code is 0: `PASS`
- If exit code is non-zero: `FAIL` -- show the output and ask:
  "Formatting issues found. Auto-fix with `cargo fmt --all`? (y/n)"
  If user approves, run `cargo fmt --all` and re-check.

## Step 3: Lint Check

```bash
cargo clippy --all-targets --all-features -- -D warnings 2>&1
```

- If exit code is 0: `PASS`
- If exit code is non-zero: `FAIL` -- show each warning with file:line information and ask:
  "Clippy found warnings. Investigate and fix? (y/n)"
  If user approves, read each flagged file and apply fixes, then re-run clippy to verify.

## Step 4: Unit Tests

```bash
cargo test --lib 2>&1
```

- If exit code is 0: `PASS` -- report the number of tests that passed
- If exit code is non-zero: `FAIL` -- show the failing tests and ask:
  "Unit tests failed. Investigate? (y/n)"
  If user approves, read the failing test code and related source to diagnose the issue.

## Step 5: Integration Tests

```bash
cargo test --test integration_test -- --test-threads=1 2>&1
```

- If exit code is 0: `PASS` -- report the number of tests that passed
- If exit code is non-zero: `FAIL` -- show the failing tests and ask:
  "Integration tests failed. Investigate? (y/n)"
  If user approves, read the test and related source code to diagnose.

## Step 6: Security Audit

Check if `cargo-audit` is installed:
```bash
command -v cargo-audit 2>/dev/null || cargo audit --version 2>/dev/null
```

If installed:
```bash
cargo audit 2>&1
```
- If exit code is 0: `PASS`
- If vulnerabilities found: `WARN` -- show the vulnerabilities and note which are actionable

If not installed: `SKIP` -- note "cargo-audit not installed. Install with: `cargo install cargo-audit`"

## Step 7: Summary

Present the full results:

```
Pre-commit checks:

  [PASS]  External references -- Clean
  [PASS]  cargo fmt -- Passed
  [FAIL]  cargo clippy -- 2 warnings
  [PASS]  Unit tests -- 45 passed
  [PASS]  Integration tests -- 12 passed
  [SKIP]  cargo audit -- Not installed

Next steps:
  1. Fix clippy warnings (listed above)
  2. Install cargo-audit: cargo install cargo-audit
  3. Re-run /pre-commit after fixes
```

If all checks pass:
```
Pre-commit checks:

  [PASS]  External references -- Clean
  [PASS]  cargo fmt -- Passed
  [PASS]  cargo clippy -- Clean
  [PASS]  Unit tests -- 45 passed
  [PASS]  Integration tests -- 12 passed
  [PASS]  cargo audit -- No vulnerabilities

All checks passed. Ready to commit.
```
