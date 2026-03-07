# Review PR

Perform a thorough code review of a pull request using specialist agents, then optionally post the review to GitHub.

Usage: `/review-pr [pr-url-or-number]`

---

## Step 1: Parse PR Identifier

Parse the argument `$ARGUMENTS` to determine the PR number:

- **URL provided** (e.g., `https://github.com/owner/repo/pull/123`): Extract owner, repo, and number from the URL
- **Number provided** (e.g., `123`): Use directly. Derive owner/repo from: `gh repo view --json owner,name --jq '.owner.login + "/" + .name'`
- **Nothing provided**: Auto-detect from current branch:
  ```bash
  gh pr list --head $(git branch --show-current) --state open --json number --jq '.[0].number'
  ```
- **No PR found**: Inform the user "No open PR found for the current branch." and stop

Store `{owner}`, `{repo}`, and `{number}`.

## Step 2: Fetch PR Metadata

```bash
gh pr view {number} --json headRefName,baseRefName,title,state,isDraft,number,url,additions,deletions,changedFiles
```

- If state is `MERGED`: inform user "PR #{number} is already merged." and stop
- If state is `CLOSED`: inform user "PR #{number} is closed." and stop
- If `isDraft` is true: warn "This PR is still a draft. Proceeding with review anyway."

Display PR summary:
```
PR #{number}: {title}
Branch: {headRefName} -> {baseRefName}
Changed files: {changedFiles} (+{additions}, -{deletions})
```

## Step 3: Checkout PR Branch

- Save current branch: `git branch --show-current` (store as `{original_branch}`)
- If not already on the PR branch:
  ```bash
  gh pr checkout {number}
  ```
  This handles both fork-based and same-repo PRs.
- Get baseSha:
  ```bash
  gh pr view {number} --json baseRefOid --jq '.baseRefOid'
  ```

## Step 4: Check for Trivial Changes

Evaluate the change scope before running full analysis:

- If ALL changed files are `.md` files: "This PR only changes documentation. Skip detailed review? (y/n)"
- If only `Cargo.lock` changed: "This PR only updates the lock file. Skip review? (y/n)"
- If user chooses to skip: provide a brief summary and stop

## Step 5: Get Changed Files

```bash
git diff --name-status {baseSha}...HEAD
```

Categorize files by type:
- Rust source files (`.rs`)
- Config files (`Cargo.toml`, `.yml`, `.yaml`, `.json`)
- Documentation (`.md`)
- Test files (files in `tests/` or containing `#[cfg(test)]`)
- Claude/agent files (`.claude/` directory)

## Step 6: Spawn Analysis Agent

Use the `analysis-agent` (from `.claude/agents/analysis-agent.md`) to determine which specialists are needed.

Provide:
- The baseSha
- The categorized file list
- The diff summary

The analysis agent returns recommendations on which specialists to invoke.

## Step 7: Spawn Specialists in Parallel

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
- Instructions to follow the shared review workflow guidelines

## Step 8: Validate Findings

Follow the validation process from `.claude/shared/review-workflow.md`:

1. **Check CLAUDE.md guidelines** -- Is the flagged pattern documented as intentional? (e.g., `unsafe` transmute in `analyzer.rs`)
2. **Spot-check the code** -- Read each file with the Read tool, verify claims match reality, confirm line numbers
3. **Assess project applicability** -- Is this a real problem for this project's scale and usage?
4. **Deduplicate** -- Multiple specialists may flag the same issue; keep the most specific version
5. **Classify** -- Relevant, Needs User Input, or Not Relevant (filter out)

## Step 9: Present Structured Report

First show the PR header:
```
PR #{number}: {title}
Branch: {headRefName} -> {baseRefName}
Changed files: {changedFiles} (+{additions}, -{deletions})
```

Then use the exact format defined in `.claude/shared/review-workflow.md` for the severity-grouped report, filtered issues transparency, and post-validation options.

## Step 10: Offer to Post Review to GitHub

Ask: "Post this review to GitHub? (y/n)"

If yes:

1. **Check authorship**: Determine if the current user authored the PR:
   ```bash
   gh pr view {number} --json author --jq '.author.login'
   gh api user --jq '.login'
   ```
   If the user is the PR author: they cannot submit `REQUEST_CHANGES` on their own PR. Use `COMMENT` event instead.

2. **Build the review body** from the validated findings summary

3. **Create a pending review** (omit the `event` field to create it in PENDING state):
   ```bash
   gh api repos/{owner}/{repo}/pulls/{number}/reviews --method POST \
     -f body=""
   ```
   Extract the `review_id` from the response.

4. **Add inline comments** for each issue, using the review ID:
   ```bash
   gh api repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}/comments --method POST \
     -f path="{file_path}" \
     -F line={line_number} \
     -f side="RIGHT" \
     -f body="{comment_body}"
   ```

5. **Ask for explicit permission** before submitting:
   "Submit review as COMMENT or REQUEST_CHANGES? (or cancel)"

6. **Submit the review**:
   ```bash
   gh api repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}/events --method POST \
     -f event="{COMMENT|REQUEST_CHANGES}" \
     -f body="{review_summary}"
   ```

## Step 11: Cleanup

Switch back to the original branch:
```bash
git checkout {original_branch}
```

## Error Handling

- **`gh` not installed**: "GitHub CLI is required. Install it: https://cli.github.com"
- **Not authenticated**: "Please authenticate first: `gh auth login`"
- **PR not found**: "PR #{number} not found in this repository."
- **Network errors**: Show the error and suggest retrying
- **No changed files**: "PR has no file changes to review." and stop
