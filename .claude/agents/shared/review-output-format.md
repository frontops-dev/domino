# Review Output Format (JSONL)

Specification for specialist agent review output. All agents must produce output in this format.

## Structure

Output consists of:

1. **First line**: A `SUMMARY` line (always required)
2. **Following lines**: One JSON object per finding (JSONL format, one per line)

---

## SUMMARY Line

The first line of output is always a summary:

```
SUMMARY: assessment=NEEDS_ATTENTION files=3 positives=Clean ownership patterns|Good error handling
```

### Assessment Values

| Value | When to Use |
|-------|-------------|
| `EXCELLENT` | No issues found. Code is well-written with notable positive patterns. |
| `GOOD` | Only low-severity findings or minor suggestions. No functional concerns. |
| `NEEDS_ATTENTION` | Medium-severity findings that should be addressed before merge. |
| `CRITICAL` | Critical-severity findings. Memory safety, soundness, or correctness issues. |

### SUMMARY Fields

- `assessment` — One of the four values above
- `files` — Number of files reviewed
- `positives` — Pipe-separated list of positive observations (what the code does well)

---

## Finding Format

Each finding is a single JSON object on its own line:

```
{"s":"critical","t":"Unsafe block without safety docs","f":"src/semantic/analyzer.rs","l":42,"d":"transmute without SAFETY comment documenting the invariants that make this safe","x":"Add // SAFETY: comment explaining why the transmute is sound — document the lifetime guarantees","c":95}
```

### Field Definitions

| Field | Name | Type | Required | Description |
|-------|------|------|----------|-------------|
| `s` | severity | string | Yes | `critical`, `medium`, or `low` |
| `t` | title | string | Yes | Short title, max 60 characters |
| `f` | file | string | Yes | File path relative to project root |
| `l` | line | number | No | Line number. Omit for file-level findings. |
| `d` | description | string | Yes | Full description of the issue |
| `x` | suggestion | string | Yes | Recommended fix or action |
| `c` | confidence | number | Yes | Confidence score 0-100 (see `confidence-scoring.md`) |
| `i` | impact | boolean | No | Set to `true` when the finding is on unchanged code that is impacted by changes in the PR. Omit (do not set to `false`) when the finding is on changed code. |

---

## Severity Guidelines (Rust-Specific)

### critical

Issues that can cause undefined behavior, data loss, or security vulnerabilities:

- Undefined behavior (UB) potential
- Memory safety violations (use-after-free, double-free, buffer overflow)
- Soundness holes in public API (safe code can trigger UB)
- Data races (missing Send/Sync bounds, shared mutable state)
- Unsafe code without `// SAFETY:` documentation
- FFI boundary violations
- Panic in FFI callbacks (UB across FFI boundary)

### medium

Issues that affect correctness, maintainability, or performance:

- Non-idiomatic patterns that harm readability
- Suboptimal error handling (bare `?` without context, `unwrap()` in production paths)
- Unnecessary allocations or clones
- Missing test coverage for new functionality
- Logic errors that do not cause UB but produce wrong results
- Missing `#[must_use]` on important Result-returning functions

### low

Suggestions and minor improvements:

- Style preferences beyond `cargo fmt`
- Documentation gaps
- Minor performance optimizations
- Alternative API designs
- Naming improvements

---

## Rules

1. **One JSON object per line** — This is JSONL format, not a JSON array. No wrapping `[]`.
2. **No trailing commas** — Each line is a standalone JSON object.
3. **All strings must be properly escaped** — Escape quotes, backslashes, newlines within JSON strings.
4. **SUMMARY is always first** — Even if there are zero findings, output the SUMMARY line.
5. **Reference `agent-guidelines.md`** for what to include and exclude from review.
6. **Every finding must include a confidence score** (`"c"` field) following the rules in `confidence-scoring.md`.

---

## Examples

### Clean Review (No Issues)

```
SUMMARY: assessment=EXCELLENT files=5 positives=Strong error handling with thiserror|Efficient use of iterators|Well-documented unsafe blocks
```

### Review with Findings

```
SUMMARY: assessment=NEEDS_ATTENTION files=3 positives=Clean module structure|Good test coverage
{"s":"medium","t":"Missing error context on fallible operation","f":"src/git.rs","l":87,"d":"The `?` operator is used without .context() or .map_err(), making it hard to diagnose failures in git diff parsing","x":"Add .context(\"Failed to parse git diff output\") using anyhow::Context","c":82}
{"s":"low","t":"Unnecessary clone of PathBuf","f":"src/core.rs","l":215,"d":"path.clone() is called but the owned value is not used after this point — the clone is unnecessary","x":"Remove .clone() and pass the owned PathBuf directly","c":75}
{"s":"critical","t":"Unsound transmute in public function","f":"src/semantic/analyzer.rs","l":142,"d":"transmute is used to extend a lifetime but the source reference can be invalidated if the caller drops the allocator","x":"Ensure the allocator is stored alongside the transmuted reference, or use a self-referential struct pattern","c":91,"i":true}
```
