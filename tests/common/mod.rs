//! Shared scaffolding for the `tests/fixtures/monorepo` fixture used by the
//! integration and CLI test binaries.
//!
//! The fixture is generated from scratch (files + git repo) the first time a
//! test needs it. It is intentionally NOT committed to the outer repository:
//! the fixture must contain its own `.git` directory (tests create branches
//! and commits inside it), and a nested `.git` would turn the directory into
//! a gitlink/phantom submodule in the outer repo — which is exactly how the
//! original fixture was lost.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Base fixture layout: an Nx-style monorepo with three projects.
///
/// - proj2 statically imports `proj1` from proj1 (and re-exports it)
/// - proj3 statically imports `anotherFn` from proj2 and declares an implicit
///   dependency on proj1 (in project.json, for CLI discovery; the in-process
///   test configs mirror it)
/// - `unusedFn` in proj1 is imported by nobody — several tests rely on that
/// - lazy-loader.tsx / page-wrapper.tsx / mixed-imports.ts / dynamic-loader.tsx
///   are dynamic-import baselines that tests guard-assert on; their exact
///   `import('@monorepo/...')` substrings must be preserved
const FIXTURE_FILES: &[(&str, &str)] = &[
  ("nx.json", "{}\n"),
  (
    "tsconfig.base.json",
    r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@monorepo/proj1": ["proj1/index.ts"],
      "@monorepo/proj1/*": ["proj1/*"],
      "@monorepo/proj2": ["proj2/index.ts"],
      "@monorepo/proj2/*": ["proj2/*"],
      "@monorepo/proj3": ["proj3/index.ts"],
      "@monorepo/proj3/*": ["proj3/*"]
    }
  }
}
"#,
  ),
  (
    "tsconfig.json",
    r#"{
  "extends": "./tsconfig.base.json"
}
"#,
  ),
  (
    "proj1/project.json",
    r#"{
  "name": "proj1",
  "sourceRoot": "proj1",
  "targets": {
    "build": {
      "options": {
        "tsConfig": "proj1/tsconfig.json"
      }
    }
  }
}
"#,
  ),
  (
    "proj1/tsconfig.json",
    r#"{
  "extends": "../tsconfig.base.json"
}
"#,
  ),
  (
    "proj1/index.ts",
    r#"export function proj1() {
  return 'proj1';
}

export function unusedFn() {
  return 'unusedFn';
}
"#,
  ),
  (
    "proj2/project.json",
    r#"{
  "name": "proj2",
  "sourceRoot": "proj2",
  "targets": {
    "build": {
      "options": {
        "tsConfig": "proj2/tsconfig.json"
      }
    }
  }
}
"#,
  ),
  (
    "proj2/tsconfig.json",
    r#"{
  "extends": "../tsconfig.base.json"
}
"#,
  ),
  (
    "proj2/index.ts",
    r#"import { proj1 } from '@monorepo/proj1';

export { proj1 } from '@monorepo/proj1';

export function proj2() {
  proj1();
  return 'proj2';
}

export function anotherFn() {
  return 'anotherFn';
}

const Decorator = () => (target: typeof MyClass) => target;

@Decorator()
export class MyClass {
  constructor() {
    proj1();
  }
}
"#,
  ),
  (
    "proj2/lazy-loader.tsx",
    r#"import { lazy } from 'react';

export const LazyProj1 = lazy(() => import('@monorepo/proj1'));
"#,
  ),
  (
    "proj2/page-wrapper.tsx",
    r#"import { lazy } from 'react';

const Proj1Page = lazy(() => import('@monorepo/proj1'));

export function PageWrapper() {
  return Proj1Page;
}
"#,
  ),
  (
    "proj2/mixed-imports.ts",
    r#"import { proj1 } from '@monorepo/proj1';

export function useStaticProj1() {
  return proj1();
}

export async function useDynamicProj1() {
  const mod = await import('@monorepo/proj1');
  return mod.proj1();
}
"#,
  ),
  (
    "proj3/project.json",
    r#"{
  "name": "proj3",
  "sourceRoot": "proj3",
  "implicitDependencies": ["proj1"],
  "targets": {
    "build": {
      "options": {
        "tsConfig": "proj3/tsconfig.json"
      }
    }
  }
}
"#,
  ),
  (
    "proj3/tsconfig.json",
    r#"{
  "extends": "../tsconfig.base.json"
}
"#,
  ),
  (
    "proj3/index.ts",
    r#"import { anotherFn } from '@monorepo/proj2';

export function proj3() {
  return anotherFn();
}
"#,
  ),
  (
    "proj3/dynamic-loader.tsx",
    r#"export async function loadProj1() {
  const mod = await import('@monorepo/proj1');
  return mod.proj1();
}

export async function loadProj2() {
  const mod = await import('@monorepo/proj2');
  return mod.proj2();
}
"#,
  ),
  // Keeps the directory present so tests can write assets outside any project
  ("shared-assets/.gitkeep", ""),
];

fn run_git(fixture: &Path, args: &[&str]) {
  let output = Command::new("git")
    .args(args)
    .current_dir(fixture)
    .output()
    .expect("Failed to execute git command");

  if !output.status.success() {
    panic!(
      "Fixture setup: git {} failed\nStderr: {}",
      args.join(" "),
      String::from_utf8_lossy(&output.stderr)
    );
  }
}

fn git_succeeds(fixture: &Path, args: &[&str]) -> bool {
  Command::new("git")
    .args(args)
    .current_dir(fixture)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// What git makes of the fixture directory.
enum FixtureRepo {
  /// git resolves the directory to the fixture's *own* repository.
  Own,
  /// git ran, but the directory is not the fixture's own repository.
  Foreign,
  /// git could not be executed at all.
  GitUnavailable,
}

/// Ask git which repository `fixture` belongs to.
///
/// The question has to be *identity* ("which repo is this?"), not liveness
/// ("does git work here?"). Git's repository discovery walks **up** from the
/// working directory until it finds a structurally valid gitdir, silently
/// skipping an empty or half-written `.git` rather than reporting it. Since
/// this fixture lives inside domino's own checkout, a corrupt `.git` makes the
/// walk land on domino itself — `rev-parse` and `show-ref refs/heads/main`
/// then both succeed against *that* repo, the fixture looks reusable, and
/// every later `git checkout` / `git commit` in the tests runs against the
/// real working tree.
fn classify_fixture_repo(fixture: &Path) -> FixtureRepo {
  let Ok(output) = Command::new("git")
    .args(["rev-parse", "--absolute-git-dir"])
    .current_dir(fixture)
    .output()
  else {
    return FixtureRepo::GitUnavailable;
  };

  if !output.status.success() {
    return FixtureRepo::Foreign;
  }

  // `--absolute-git-dir` is absolute but not necessarily canonical (macOS
  // resolves `/var` to `/private/var`), so compare canonicalised forms.
  let resolved = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
  match (resolved.canonicalize(), fixture.join(".git").canonicalize()) {
    (Ok(resolved), Ok(expected)) if resolved == expected => FixtureRepo::Own,
    _ => FixtureRepo::Foreign,
  }
}

fn write_fixture_files(fixture: &Path) {
  for (rel_path, content) in FIXTURE_FILES {
    let path = fixture.join(rel_path);
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent).expect("Failed to create fixture directory");
    }
    fs::write(&path, content).expect("Failed to write fixture file");
  }
}

/// Canonical content of a generated fixture file, for tests that need to
/// restore state they mutated on `main`.
pub fn fixture_file_content(rel_path: &str) -> &'static str {
  FIXTURE_FILES
    .iter()
    .find(|(path, _)| *path == rel_path)
    .map(|(_, content)| *content)
    .unwrap_or_else(|| panic!("No fixture file at {rel_path}"))
}

/// Ensure the monorepo fixture exists on disk and is a git repo with a usable
/// `main` branch. Generates everything from scratch when missing, and wipes +
/// regenerates the fixture if a previous interrupted run left the repo in a
/// broken state (e.g. HEAD on an unborn branch).
pub fn ensure_fixture_git_repo(fixture: &Path) {
  // Self-heal: an interrupted run can leave `.git` behind in a state that is
  // not reusable. `classify_fixture_repo` is what decides, because merely
  // asking whether git works here answers the wrong question — see its docs
  // for why a corrupt `.git` resolves to the *enclosing* domino repository
  // instead of failing. A `.git` *file* is covered by the same comparison:
  // linked-worktree or submodule metadata points at a gitdir elsewhere.
  if fixture.join(".git").exists() {
    match classify_fixture_repo(fixture) {
      // Reuse only when the fixture is its own repo *and* carries a local
      // `main` branch. `show-ref --verify refs/heads/main` rather than
      // `rev-parse --verify main`: the latter also resolves a *tag* named
      // `main`, which would let a fixture with no local branch skip
      // regeneration and then fail on checkout.
      FixtureRepo::Own
        if git_succeeds(
          fixture,
          &["show-ref", "--verify", "--quiet", "refs/heads/main"],
        ) =>
      {
        return;
      }
      // git could not run at all. Deleting the fixture would not help — the
      // rebuild below fails the same way — so leave it in place and let
      // `run_git` surface the real error.
      FixtureRepo::GitUnavailable => {}
      _ => fs::remove_dir_all(fixture).expect("Failed to remove broken fixture"),
    }
  }

  write_fixture_files(fixture);
  run_git(fixture, &["init"]);
  run_git(fixture, &["config", "user.email", "test@example.com"]);
  run_git(fixture, &["config", "user.name", "Test User"]);
  run_git(fixture, &["branch", "-M", "main"]);
  run_git(fixture, &["add", "."]);
  run_git(fixture, &["commit", "-m", "Initial commit"]);
}
