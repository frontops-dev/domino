use super::resolve_options::{create_resolve_options, is_workspace_specifier};
use super::tsconfig_paths::TsconfigPaths;
use crate::types::Project;
use oxc_resolver::Resolver;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

type TsconfigPathsCache = RwLock<FxHashMap<PathBuf, Option<Arc<TsconfigPaths>>>>;

/// Import resolution shared by the import index, the re-export index and the
/// reference finder, so all three agree on what a specifier points at.
///
/// An import first goes through the `paths` of the nearest `tsconfig.json` above the
/// importing file, so aliases a project declares for itself (e.g. an app's `$pages/*`)
/// reach their target. `tsconfig.base.json` alone only knows the workspace-wide ones.
/// When no such alias matches or its targets do not exist, the import resolves through
/// `tsconfig.base.json` as before.
///
/// The nearest `tsconfig.json` stands in for the one that owns the file: its `include`
/// and `exclude` are not checked. A file a stricter config would leave out still sees
/// that config's aliases, which can only add import edges.
///
/// `Sync`: the import index resolves in parallel, so both caches sit behind locks.
pub(crate) struct WorkspaceResolver {
  cwd: PathBuf,
  projects: Vec<Project>,
  root_path_prefixes: Vec<String>,
  resolver: Resolver,
  paths_by_directory: TsconfigPathsCache,
  paths_by_tsconfig: TsconfigPathsCache,
}

impl WorkspaceResolver {
  pub(crate) fn new(cwd: &Path, projects: Vec<Project>, root_path_prefixes: Vec<String>) -> Self {
    Self {
      resolver: Resolver::new(create_resolve_options(cwd, &projects)),
      cwd: cwd.to_path_buf(),
      projects,
      root_path_prefixes,
      paths_by_directory: RwLock::default(),
      paths_by_tsconfig: RwLock::default(),
    }
  }

  /// Resolve `specifier` imported from the workspace-relative `importing_file` to a
  /// workspace-relative path. `None` when the import is external, unresolvable, or
  /// resolves outside the workspace.
  pub(crate) fn resolve(&self, importing_file: &Path, specifier: &str) -> Option<PathBuf> {
    let from_path = self.cwd.join(importing_file);
    let context = from_path.parent()?;

    if let Some(paths) = self
      .nearest_tsconfig_paths(context)
      .filter(|_| !is_relative_specifier(specifier))
    {
      let aliased = paths
        .candidates(specifier)
        .into_iter()
        .find_map(|candidate| {
          let resolution = self.resolver.resolve(context, candidate.to_str()?).ok()?;
          self.workspace_relative(resolution.path())
        });
      if aliased.is_some() {
        return aliased;
      }
    }

    if !is_workspace_specifier(specifier, &self.projects, &self.root_path_prefixes) {
      return None;
    }

    match self.resolver.resolve(context, specifier) {
      Ok(resolution) => self.workspace_relative(resolution.path()),
      Err(_) => super::simple_resolve_relative(&self.cwd, context, specifier),
    }
  }

  fn workspace_relative(&self, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(&self.cwd).ok().map(Path::to_path_buf)
  }

  fn nearest_tsconfig_paths(&self, directory: &Path) -> Option<Arc<TsconfigPaths>> {
    if let Some(cached) = read_cache(&self.paths_by_directory, directory) {
      return cached;
    }

    let paths = directory
      .ancestors()
      .take_while(|ancestor| ancestor.starts_with(&self.cwd))
      .map(|ancestor| ancestor.join("tsconfig.json"))
      .find(|tsconfig| tsconfig.is_file())
      .and_then(|tsconfig| self.load_tsconfig_paths(tsconfig));

    write_cache(
      &self.paths_by_directory,
      directory.to_path_buf(),
      paths.clone(),
    );
    paths
  }

  fn load_tsconfig_paths(&self, tsconfig: PathBuf) -> Option<Arc<TsconfigPaths>> {
    if let Some(cached) = read_cache(&self.paths_by_tsconfig, &tsconfig) {
      return cached;
    }
    let paths = TsconfigPaths::load(&tsconfig, &self.cwd, &self.projects).map(Arc::new);
    write_cache(&self.paths_by_tsconfig, tsconfig, paths.clone());
    paths
  }
}

/// TypeScript's `pathIsRelative`: `./x`, `../x`, `.` and `..` resolve from the importing
/// file and never through `paths`, even a catch-all `"*"` key.
fn is_relative_specifier(specifier: &str) -> bool {
  specifier
    .strip_prefix("..")
    .or_else(|| specifier.strip_prefix('.'))
    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
}

/// A poisoned lock only means another thread panicked mid-insert; the map itself is
/// still a valid cache, so keep using it.
fn read_cache(cache: &TsconfigPathsCache, key: &Path) -> Option<Option<Arc<TsconfigPaths>>> {
  cache
    .read()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .get(key)
    .cloned()
}

fn write_cache(cache: &TsconfigPathsCache, key: PathBuf, value: Option<Arc<TsconfigPaths>>) {
  cache
    .write()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
    .insert(key, value);
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use tempfile::TempDir;

  #[test]
  fn test_paths_skip_relative_specifiers() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().canonicalize().unwrap();
    fs::create_dir_all(cwd.join("src")).unwrap();
    fs::create_dir_all(cwd.join("types")).unwrap();
    fs::write(
      cwd.join("tsconfig.json"),
      r#"{ "compilerOptions": { "paths": { "*": ["./types/*"] } } }"#,
    )
    .unwrap();
    fs::write(cwd.join("src/foo.ts"), "export const foo = 1;").unwrap();
    fs::write(cwd.join("types/foo.ts"), "export const foo = 2;").unwrap();
    fs::write(cwd.join("types/react.ts"), "export const react = 1;").unwrap();

    let resolver = WorkspaceResolver::new(&cwd, vec![], vec![]);

    assert_eq!(
      resolver.resolve(Path::new("src/app.ts"), "./foo"),
      Some(PathBuf::from("src/foo.ts")),
      "a relative import resolves from the importing file, never through `paths`"
    );
    assert_eq!(
      resolver.resolve(Path::new("src/app.ts"), "react"),
      Some(PathBuf::from("types/react.ts")),
      "a bare specifier still goes through `paths`"
    );
  }
}
