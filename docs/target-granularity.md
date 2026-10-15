# Target granularity: what Domino should and should not do

Domino answers "which **projects** are affected". Nx is moving toward answering "which **tasks** are affected" (`nrwl/nx#36841`, merged into the stacked branch behind `nrwl/nx#36825`, which is not yet in `master`). This document records what Domino will do about that, and what it deliberately will not.

## Recommendation

1. **Document the intersection recipe (no code).** Intersect Nx's target-filtered affected set with Domino's project set. It works against released Nx today and keeps working if task-grained selection lands.
2. **Add an affected-files output as the enabling primitive.** Surface the set of files Domino considers affected (a CLI flag and an N-API field). It is mostly plumbing, and it is the prerequisite for anything task-shaped later.

Everything else in this space is ruled out below, with the reason.

## 1. The intersection recipe

`nx show projects --affected -t <target>` lists the projects Nx considers affected **and** that define `<target>`. Domino's list is the semantically affected projects. A project needs `<target>` run only if it is in both:

```bash
nx show projects --affected -t test --json | jq -r '.[]' | sort > nx.txt
domino affected --json | jq -r '.[]' | sort > domino.txt
projects=$(comm -12 nx.txt domino.txt | paste -sd, -)
[ -n "$projects" ] && nx run-many -t test -p "$projects"
```

Why this is safe to recommend:

- Domino's set is a refinement of project-level affected: it drops projects that are file-affected but not semantically affected. Nx's `-t` filter drops projects that do not define the target (and, once task-grained selection ships, targets whose inputs did not change). The two filters are independent, so the intersection keeps both refinements.
- It needs nothing unreleased from Nx. If `nrwl/nx#36825` lands, the `nx show projects --affected -t` half simply gets finer and the recipe stays the same.

## 2. Affected-files output

The report already knows the files behind every decision: every file-driven `AffectCause` carries a file path (`src/types.rs`: `DirectChange`, `ImportedSymbol`, `AssetChange`, ...), and `ReportTotals::changed_files` exists but is only a count. Exposing the set is the smallest step that makes task-shaped consumers possible:

- CLI: an `--affected-files` flag, so the current `--json` array output does not change shape.
- N-API: an optional `affectedFiles` field on the result.

The set is useful on its own (test runners that accept file lists, custom task filters) and is what any later per-target work would consume. It needs no new analysis, only collection of paths the pipeline already visits.

## Ruled out

### Feeding Domino's affected files to `nx affected --files`

Tempting, because it looks like task granularity for free. It is not. A dependent's task inputs normally include its dependencies' filesets through `^production` (or `^default`), so any file inside a dependency re-expands to every dependent of that dependency, which is exactly the set Domino just narrowed. The round trip gives back project-level affected and throws away the semantic result.

### Reimplementing Nx's task selection in Domino

Nx's task-grained selection stands on three pieces Domino does not have and should not grow:

- **Hash plans** (`HashPlanner`), which resolve each task's `inputs` (named inputs, `^` dependency inputs, runtime and external inputs) into concrete filesets.
- **The task graph** (`createTaskGraph`), which expands `dependsOn` across projects and targets.
- **An I/O snapshot** of the workspace that the planner hashes against.

Rebuilding these in Rust would mean re-deriving Nx's semantics on every Nx release, with no way to stay in lockstep. That is the opposite of Domino's job, which is a precise, semantic project set that Nx can then filter.

## What Domino does take from the same PR

Reading `nrwl/nx#36841` alongside Domino surfaced places where Domino under-selects on its own terms. They are tracked as separate fixes, not as parity work:

- A renamed file never marked the project it moved out of.
- A change to the workspace-root `tsconfig.base.json` (the tsconfig Domino's resolution reads) affected nothing.
- A glob-free `namedInputs` path (`{workspaceRoot}/config`) matched only a file with that exact name, not the directory's contents.
- Lockfile diffing compared versions only, missing integrity-only changes.
- Project links declared in target config (`inputs[].projects`, `dependsOn[].projects`, and the same shapes in `targetDefaults`) created no edge.

## Sources

- [nrwl/nx#36841: feat(core): select affected tasks instead of affected projects](https://github.com/nrwl/nx/pull/36841) (merged 2026-09-17 into `feat/nxc-4859-task-based-affected`)
- [nrwl/nx#36825: the same change targeting `master`](https://github.com/nrwl/nx/pull/36825) (open at the time of writing)
- [Nx docs: `nx show projects`](https://nx.dev/reference/nx-commands#show-projects)
- [Nx docs: configuring inputs](https://nx.dev/recipes/running-tasks/configure-inputs)
- Domino source: `src/types.rs` (`AffectCause`, `ReportTotals`), `src/cli.rs` (`--json` output)
