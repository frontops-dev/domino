# Review Workflow

Shared validation and presentation workflow used by commands like `/review-pr`, `/resolve-pr-comments`, and `/review-branch`. Defines how specialist findings are validated, filtered, and presented to the user.

## Issue Validation Pass

For each issue reported by a specialist, the orchestrating command must validate:

1. **Check CLAUDE.md guidelines**
   - Does the project's CLAUDE.md accept this pattern? (e.g., `unsafe` transmute in analyzer.rs is documented and intentional)
   - Is there a project convention that makes this acceptable?

2. **Spot-check the code**
   - Read the file with Read tool. Verify the claim matches reality
   - For claims about changes, verify against `git diff ${baseSha}...HEAD -- {file}`
   - Confirm line numbers are correct

3. **Assess project applicability**
   - Is this a real problem in this specific project, or a generic best practice?
   - Would this matter given the project's scale and usage?

4. **Deduplicate**
   - Multiple specialists may flag the same issue from different angles
   - Keep the most specific/actionable version, remove duplicates

5. **Classify**
   - **Relevant** -- Real issue, include in report
   - **Needs User Input** -- Ambiguous, present with context for user decision
   - **Not Relevant** -- False positive, pre-existing, or already handled; filter out

## Presenting Validated Issues

Use this shell output format with severity groupings:

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

▶ CRITICAL ISSUES (count)

#1 Issue Title -- `path/to/file.rs:42`
**Why this matters:** [Explain relevance to THIS project, not generic best practices]
**Suggestion:**
\`\`\`rust
// suggested fix
\`\`\`

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

▶ MEDIUM ISSUES (count)

#2 Issue Title -- `path/to/file.rs:88`
**Why this matters:** [...]
**Suggestion:** [...]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

▶ LOW ISSUES (count)

#3 Issue Title — `path/to/file.rs:120`
**Suggestion:** [...]

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

▶ WHAT'S DONE WELL

- Clean ownership patterns in core.rs
- Good error handling with thiserror
- Well-structured tests

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Mark issues found in unchanged code (impacted by changes) with `[OUTSIDE CHANGE]`.

## Writing Style Rules

- Be direct: "This should use `?` instead of `unwrap()`" not "It might be beneficial to consider..."
- Use natural language like a colleague in code review
- NO em-dashes mid-sentence in GitHub comments; use periods or commas instead
- NO templated phrases: "It's worth noting that...", "This review covers..."
- Varied sentence structure. Don't start every point the same way
- Skip hedging: avoid "perhaps", "might be beneficial", "could potentially"
- Reference specific code, not abstract concepts

## Filtered Issues Transparency

Show what was filtered and why:

```
Filtered X issues:
- "Unnecessary clone in core.rs:55" -- Pre-existing (verified via git show)
- "Missing docs on process_files" -- Clippy would catch this
- "unsafe in analyzer.rs:189" -- Documented transmute per CLAUDE.md
```

## Post-Validation Options

After presenting the report, offer:
1. Fix all critical/medium issues automatically
2. Post review to GitHub (for PR reviews)
3. Show filtered issues (transparency)
4. Skip and continue

## Available Specialists

| Specialist | Scope | When to Use |
|---|---|---|
| `rust-specialist` | Ownership, lifetimes, error handling, performance, idioms, Cargo | All `.rs` and `Cargo.toml` changes |
| `security-specialist` | Unsafe code, FFI, supply chain, input validation | Files with `unsafe`, FFI, new deps |
| `test-specialist` | Test coverage, quality, patterns | Test files and source without tests |
| `ai-specialist` | Agent definitions, commands, prompt quality | `.claude/` directory changes |

## Preventing Hallucinations

**Mandatory verification rules** (these CANNOT be skipped):
1. Never report an issue without reading the file first
2. Quote exact code from the file, not from memory
3. Verify line numbers by reading the file with offset
4. Check if pre-existing: `git show ${baseSha}:path/to/file`
5. If verification reveals "this is NOT an issue", delete it entirely before presenting
