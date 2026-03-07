# Confidence Scoring Guide

Every finding must include a confidence score (`"c"` field) in the JSONL output. This guide defines the scale, provides Rust-specific examples, and lists the verification checklist.

## Scale

| Range | Label | Meaning |
|-------|-------|---------|
| **90-100** | Definitive | Certain this is a real issue. Evidence is clear and unambiguous. |
| **70-89** | Very Likely | Strong evidence, high confidence. Minor possibility of a valid justification. |
| **50-69** | Probable | Reasonable evidence, but some uncertainty remains. May depend on context not visible in the diff. |
| **30-49** | Possible | Limited evidence, notable uncertainty. The issue may be intentional or mitigated elsewhere. |
| **0-29** | Low | Speculative or circumstantial only. Likely a false positive or already handled. |

---

## Rust-Specific Examples Per Tier

### 90-100 (Definitive)

These are clear, verifiable issues with strong evidence:

- Unsafe code without `// SAFETY:` documentation
- Use-after-free potential (raw pointer dereference after drop)
- Missing `Send`/`Sync` bounds on types used across threads
- `unwrap()` on user-facing error paths in production code (not tests)
- Buffer overflow potential in FFI boundaries (N-API bindings)
- Soundness holes in public API (safe code can trigger UB)
- Panic inside `extern "C"` functions (UB at FFI boundary)
- Data race from `Rc`/`RefCell` shared across threads

### 70-89 (Very Likely)

Strong evidence but the author may have a valid reason:

- Unnecessary `.clone()` where borrowing would suffice
- Missing error context (bare `?` without `.context()` or `.map_err()`)
- Suboptimal lifetime bounds (overly restrictive)
- Non-idiomatic error types (`String` errors instead of `thiserror` enums)
- Missing `#[must_use]` on `Result`-returning functions
- Redundant allocation (`String::from()` where `&str` works)
- Missing `Drop` impl for types managing resources

### 50-69 (Probable)

Reasonable concern but context-dependent:

- Manual loop where iterator chain would be more idiomatic
- Missing `#[inline]` on small hot-path functions
- Could use `Cow<str>` instead of owned `String`
- Allocation in a loop that could be hoisted
- Missing documentation on public API items
- Potential performance issue (depends on call frequency)
- Error variant that seems unused

### 30-49 (Possible)

Limited evidence, may be intentional:

- Naming convention preferences (specific `snake_case` style choices)
- Module organization suggestions
- Minor documentation improvements
- Alternative crate suggestions (e.g., suggesting a different error crate)
- Function could be `const` but is not
- Dead code that might be used in future

### 0-29 (Low)

Likely not actionable:

- Clippy would catch this automatically (redundant with CI)
- Pre-existing issue not introduced by this change
- Pure style preference with no functional impact
- Suggestion contradicted by project conventions
- Issue in generated or macro-expanded code

---

## Pre-Assignment Verification Checklist

Before assigning a confidence score, answer these five questions:

### 1. Did you READ the actual code?

You must have used the Read tool to view the file at the relevant lines. Do not assign confidence based solely on diff output.

- If No: Do not report the finding. Read the file first.

### 2. Did you verify the line numbers match?

Diff hunk headers can be misleading. Confirm the line number in the current HEAD version of the file.

- If No: Re-read the file and correct line numbers before scoring.

### 3. Is this pre-existing?

Check with `git show ${baseSha}:path/to/file.rs`. If the same issue existed before this PR, do not report it (score would be 0).

- If Yes (pre-existing): Remove the finding entirely. Do not report it.
- Exception: If the change makes a pre-existing issue worse or exposes it to new callers, report it with `"i": true`.

### 4. Is there a `#[allow()]` or documented justification?

Check for `#[allow(clippy::...)]`, `// SAFETY:` comments, or documented rationale near the code.

- If Yes: Reduce confidence by 30-50 points or remove the finding.

### 5. Would `cargo clippy` already catch this?

If the issue is something `cargo clippy` detects, it is redundant to report it manually (CI will catch it).

- If Yes: Score 0-29 at most. Consider removing the finding.

---

## Output Format

Include the confidence score in every finding using the `"c"` field:

```
{"s":"medium","t":"Missing error context","f":"src/git.rs","l":87,"d":"Bare ? operator without context","x":"Add .context()","c":82}
```

The `"c"` value must be an integer between 0 and 100. Do not use ranges or qualitative labels in the field — use the integer score only.
