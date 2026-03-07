# Create PR

Create a pull request from the current branch with an auto-generated title and description.

Usage: `/pr-create [base-branch]`

Default base branch: `main`

---

## Step 1: Verify State

Determine the base branch from `$ARGUMENTS`, defaulting to `main` if not provided.

```bash
git branch --show-current
```

- If on `main` (or the specified base branch): inform user "You're on the base branch. Switch to a feature branch first." and stop

Check for commits beyond base:
```bash
git log {base}..HEAD --oneline
```
- If no commits: "Branch has no commits beyond {base}." and stop

Check working directory:
```bash
git status --porcelain
```
- If uncommitted changes exist: warn "You have uncommitted changes. They will NOT be included in the PR. Continue? (y/n)"

## Step 2: Analyze Changes

Gather context for generating the PR content:

```bash
git log {base}..HEAD --oneline
```

```bash
git diff --stat {base}...HEAD
```

```bash
git diff --name-status {base}...HEAD
```

Use the commit messages and file change summary to understand the intent. If you need to understand specific file changes in detail, read individual files with the Read tool rather than fetching the full diff (which can cause memory issues on large PRs).

## Step 3: Generate PR Content

### Title

- Follow conventional commits format: `fix:`, `feat:`, `refactor:`, `perf:`, `test:`, `docs:`, `chore:`
- Keep under 70 characters
- Derive from commit messages and actual code changes
- Use lowercase after the prefix

Examples:
- `feat: add barrel file re-export detection`
- `fix: handle missing tsconfig.base.json gracefully`
- `refactor: extract module resolution into dedicated struct`

### Description

Use this template:

```markdown
## Summary
- [2-4 bullet points describing what changed and why]

## Changes
- [Key changes organized by file or area]

## Testing
- [ ] Unit tests pass (`cargo test --lib`)
- [ ] Integration tests pass (`cargo test --test integration_test -- --test-threads=1`)
- [ ] Clippy clean (`cargo clippy --all-targets --all-features`)
- [ ] Formatted (`cargo fmt --all`)
```

Add additional testing notes if relevant (e.g., "Tested against a monorepo with 200+ projects").

## Step 4: Apply Obfuscation Rules

Before creating the PR, scan the generated title and description for violations of the CLAUDE.md obfuscation rules:

- External repository names or organization names
- Customer or client project names
- Real file paths from external projects

If any are found:
- Replace with generic names (e.g., "app-client", "Component.tsx", "getHelperValue()")
- Warn the user about what was replaced

Also scan the diff itself -- if commit messages contain external references, warn the user.

## Step 5: Present for Approval

Show the user the generated PR content:

```
PR Title: feat: add barrel file re-export detection

PR Description:
## Summary
- Added support for detecting re-exports through barrel files (index.ts)
- Symbols re-exported via barrel files now correctly propagate affected status

## Changes
- src/semantic/reference_finder.rs: Added barrel file traversal logic
- src/semantic/analyzer.rs: Extended import index to track re-exports
- tests/integration_test.rs: Added test for barrel file scenarios

## Testing
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Clippy clean
- [ ] Formatted

Base branch: main
```

Ask: "Create this PR? (y/n) — or edit title/description"

## Step 6: Push and Create PR

If approved:

```bash
git push -u origin {current_branch}
```

```bash
gh pr create --title "{title}" --body "$(cat <<'EOF'
{description}
EOF
)"  --base {base}
```

## Step 7: Return Result

Display the created PR URL to the user:

```
PR created: https://github.com/{owner}/{repo}/pull/N
```

## Error Handling

- **`gh` not installed**: "GitHub CLI is required. Install it: https://cli.github.com"
- **Not authenticated**: "Please authenticate first: `gh auth login`"
- **Push rejected**: Show the error. Suggest `git pull --rebase origin {branch}` if behind remote.
- **PR already exists**: "A PR already exists for this branch: {url}. Update it instead? (y/n)"
  If yes, use `gh pr edit {number}` to update title and body.
