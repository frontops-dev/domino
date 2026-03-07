# Resolve PR Comments

Resolve unresolved PR review comment threads by fixing code, drafting replies, and posting them to GitHub.

Usage: `/resolve-pr-comments [pr-url-or-number]`

---

## Step 1: Parse PR Identifier

Parse the argument `$ARGUMENTS` to determine the PR number:

- **URL provided** (e.g., `https://github.com/owner/repo/pull/123`): Extract owner, repo, and number from the URL
- **Number provided** (e.g., `123`): Use directly. Derive owner/repo from the current git remote: `gh repo view --json owner,name --jq '.owner.login + "/" + .name'`
- **Nothing provided**: Auto-detect from current branch:
  ```bash
  gh pr list --head $(git branch --show-current) --state open --json number --jq '.[0].number'
  ```
- **No PR found**: Inform the user "No open PR found for the current branch." and stop

Store the resolved `{owner}`, `{repo}`, and `{number}` for all subsequent steps.

## Step 2: Fetch PR Metadata

Run:
```bash
gh pr view {number} --json headRefName,baseRefName,title,state,isDraft,number,url
```

- If state is `CLOSED` or `MERGED`: inform user and stop
- Save the current branch: `git branch --show-current` (store as `{original_branch}`)
- If not already on the PR branch (`headRefName`):
  ```bash
  git fetch origin {headRefName} && git checkout {headRefName}
  ```
- Get baseSha:
  ```bash
  gh pr view {number} --json baseRefOid --jq '.baseRefOid'
  ```

## Step 3: Fetch Unresolved Comment Threads via GraphQL

**CRITICAL**: The REST API does not expose thread resolution status. You MUST use the GraphQL API.

```bash
gh api graphql -f query='
query($owner: String!, $repo: String!, $pr: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $pr) {
      reviewThreads(first: 100) {
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          startLine
          diffSide
          comments(first: 50) {
            nodes {
              id
              body
              author { login }
              createdAt
              path
              position
              originalPosition
            }
          }
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}' -f owner='{owner}' -f repo='{repo}' -F pr={number}
```

- Filter to threads where `isResolved` is `false` only
- Group the unresolved threads by file path
- If zero unresolved threads: inform user "All review threads are resolved!" and stop
- If `hasNextPage` is true: paginate using `endCursor` by adding `-f after='{endCursor}'` and `reviewThreads(first: 100, after: $after)` to the query
- If more than 50 unresolved threads: warn the user about the large count and ask whether to proceed

## Step 4: Classify Each Comment Thread

For each unresolved thread:

1. Read the file at the path indicated by the thread using the Read tool
2. Read the comment body and all replies in the thread for full context
3. Classify into one of:

| Classification | Description | Action |
|---|---|---|
| **actionable** | Concrete code change requested by the reviewer | Fix the code |
| **question** | Reviewer is asking for clarification or rationale | Draft an explanation reply |
| **nitpick** | Minor style or preference suggestion | Fix if trivial, explain if deliberate |
| **already-addressed** | The code has already been fixed but the thread was not resolved | Verify the fix exists, draft reply pointing to it |
| **outdated** | The code no longer exists or was significantly refactored away | Draft reply noting the change |

Present the classification summary to the user:

```
Found X unresolved comment threads on PR #N:

  Y actionable (code changes needed)
  Z questions (replies needed)
  W nitpicks
  V already-addressed
  U outdated

Proceed with fixing actionable items and drafting replies? (y/n)
```

Wait for user confirmation before proceeding.

## Step 5: Fix Actionable Items

For each thread classified as **actionable** or fixable **nitpick**:

1. Read the file at the referenced location using the Read tool
2. Understand what the reviewer is requesting from the comment body
3. Apply the fix using the Edit tool
4. If the file is a Rust file (`.rs`):
   - Run `cargo clippy --all-targets --all-features` to verify no new warnings
   - Run `cargo fmt --all` to ensure formatting
5. For non-Rust files (`.toml`, `.md`, `.yml`, `.yaml`, `.json`): apply fixes directly
6. Track every change made (file, line, description) for the commit message

## Step 6: Draft Replies

Draft a reply for EVERY unresolved thread, regardless of classification:

- **actionable**: "Fixed -- [brief description of the change applied]."
- **question**: Read the surrounding code context and draft a clear, direct explanation answering the reviewer's question
- **nitpick** (fixed): "Fixed."
- **nitpick** (kept): "Kept as-is because [concrete reason]."
- **already-addressed**: "This was addressed -- the current code at line X now [description of current state]."
- **outdated**: "This code was changed/removed in a subsequent update."

Follow the writing style rules from `.claude/shared/review-workflow.md`:
- Be direct, no hedging language
- No em-dashes mid-sentence in GitHub comments; use periods or commas
- No templated filler phrases
- Reference specific code, not abstract concepts

Present ALL drafted replies to the user:

```
Draft replies:

#1 [actionable] src/core.rs:42 -- @reviewer_name
   Comment: "Should use ? instead of unwrap() here"
   Fix applied: Replaced unwrap() with proper error propagation
   Reply: "Fixed. Now uses `?` with a descriptive error context."

#2 [question] src/git.rs:88 -- @reviewer_name
   Comment: "Why is this parsing done manually instead of using a library?"
   Reply: "The git diff format is straightforward enough that..."

#3 [already-addressed] src/semantic/analyzer.rs:155 -- @reviewer_name
   Comment: "Add a safety comment for this transmute"
   Reply: "This was addressed. Line 155 now has a safety comment explaining the lifetime guarantee."
```

Ask the user: "Post these replies to GitHub? (y/n)"

## Step 7: Post Replies to GitHub

If the user approves, post each reply using the GraphQL mutation. Track successes and failures -- if a mutation fails, continue with the remaining threads and report all failures at the end:

```bash
gh api graphql -f query='
mutation($threadId: ID!, $body: String!) {
  addPullRequestReviewThreadReply(input: {pullRequestReviewThreadId: $threadId, body: $body}) {
    comment { id }
  }
}' -f threadId='{thread_id}' -f body='{reply_text}'
```

After posting replies, ask the user: "Also resolve the threads? (y/n)"

If yes, resolve threads classified as **actionable**, **already-addressed**, and fixed **nitpicks**:

```bash
gh api graphql -f query='
mutation($threadId: ID!) {
  resolveReviewThread(input: {threadId: $threadId}) {
    thread { isResolved }
  }
}' -f threadId='{thread_id}'
```

Do NOT auto-resolve **question** or **outdated** threads -- those should be resolved by the reviewer.

After all mutations complete, report results:
```
Posted N replies successfully, M failed.
Resolved K threads.
Failed: [list of thread paths and error messages, if any]
```

## Step 8: Commit and Cleanup

If any code fixes were applied in Step 5:

Present a summary:
```
Changes applied:
  - src/core.rs: Replaced unwrap() with error propagation
  - src/git.rs: Added input validation

Commit these changes? (y/n)
Suggested message: "fix: address PR review comments (#N)"
```

If user approves:
1. Stage the changed files by name (do NOT use `git add -A`)
2. Commit with the agreed message
3. Ask: "Push to origin/{headRefName}? (y/n)"
4. If yes: `git push origin {headRefName}`

Cleanup: if we switched branches in Step 2, switch back:
```bash
git checkout {original_branch}
```

## Error Handling

- **`gh` not installed**: "GitHub CLI is required. Install it: https://cli.github.com"
- **Not authenticated**: "Please authenticate first: `gh auth login`"
- **PR not found**: "PR #{number} not found in this repository."
- **GraphQL errors**: Show the error message from the API response and stop
- **File not found**: If a commented file no longer exists, classify the thread as **outdated**
- **Merge conflicts**: If the PR branch has conflicts, warn the user and stop
