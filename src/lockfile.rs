//! Lockfile change detection for npm, yarn, pnpm, and bun.
//!
//! This module detects which direct dependencies have changed between the current
//! and base lockfiles, builds a reverse dependency graph to resolve transitive
//! changes, and identifies source files that import affected packages.

use crate::error::{DominoError, Result};
use crate::types::ChangedFile;
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;
use tracing::debug;

/// Maximum lockfile size we will load into memory (256 MB).
/// Prevents OOM in memory-constrained CI when encountering very large monorepo lockfiles.
const MAX_LOCKFILE_BYTES: u64 = 256 * 1024 * 1024;

/// Supported package managers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageManager {
  Npm,
  Yarn,
  Pnpm,
  Bun,
}

/// Metadata for a single resolved package inside a lockfile.
#[derive(Debug, Clone)]
pub struct PackageInfo {
  /// Resolved version string (format varies by package manager).
  pub version: String,
  /// Direct dependencies of this package (`name → version specifier`).
  pub dependencies: FxHashMap<String, String>,
}

/// Parsed representation of a lockfile, independent of package manager format.
#[derive(Debug, Clone)]
pub struct LockfileData {
  /// Packages listed in the project's own `dependencies` / `devDependencies`.
  pub direct_dependencies: FxHashSet<String>,
  /// All resolved packages (`name → info`).
  pub packages: FxHashMap<String, PackageInfo>,
}

impl LockfileData {
  /// An empty lockfile — used as the baseline when no previous lockfile exists.
  fn empty() -> Self {
    Self {
      direct_dependencies: FxHashSet::default(),
      packages: FxHashMap::default(),
    }
  }
}

// ---------------------------------------------------------------------------
// Detection helpers
// ---------------------------------------------------------------------------

/// Auto-detect the package manager by probing for known lockfile names.
///
/// Priority: npm > yarn > pnpm > bun.  If the workspace contains multiple
/// lockfiles (e.g. during a migration) the first match wins.  Users can
/// bypass detection via `--lockfile-strategy=none`.
pub fn detect_package_manager(cwd: &Path) -> Option<PackageManager> {
  if cwd.join("package-lock.json").exists() {
    Some(PackageManager::Npm)
  } else if cwd.join("yarn.lock").exists() {
    Some(PackageManager::Yarn)
  } else if cwd.join("pnpm-lock.yaml").exists() {
    Some(PackageManager::Pnpm)
  } else if cwd.join("bun.lock").exists() {
    Some(PackageManager::Bun)
  } else {
    None
  }
}

/// Return the conventional lockfile filename for a given package manager.
pub fn lockfile_name(pm: &PackageManager) -> &'static str {
  match pm {
    PackageManager::Npm => "package-lock.json",
    PackageManager::Yarn => "yarn.lock",
    PackageManager::Pnpm => "pnpm-lock.yaml",
    PackageManager::Bun => "bun.lock",
  }
}

/// Check whether the lockfile for `pm` appears among the changed files.
pub fn has_lockfile_changed(changed_files: &[ChangedFile], pm: &PackageManager) -> bool {
  let name = lockfile_name(pm);
  changed_files
    .iter()
    .any(|f| f.file_path.to_str() == Some(name))
}

/// Read a file at `file_path` from the given git revision.
///
/// Enforces `MAX_LOCKFILE_BYTES` on the git object size before materializing
/// the content, preventing OOM when a base-revision lockfile is very large.
pub(crate) fn get_file_from_revision(
  repo_path: &Path,
  revision: &str,
  file_path: &str,
) -> Result<String> {
  let revision_path = format!("{}:{}", revision, file_path);

  // Check object size before buffering the full content.
  let size_output = Command::new("git")
    .args(["cat-file", "-s", &revision_path])
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git cat-file -s: {}", e)))?;

  if size_output.status.success() {
    if let Ok(size_str) = std::str::from_utf8(&size_output.stdout) {
      if let Ok(size) = size_str.trim().parse::<u64>() {
        if size > MAX_LOCKFILE_BYTES {
          return Err(DominoError::Other(format!(
            "Git object '{}' exceeds {} MB size limit ({} bytes)",
            revision_path,
            MAX_LOCKFILE_BYTES / (1024 * 1024),
            size,
          )));
        }
      }
    }
  }

  let output = Command::new("git")
    .args(["show", &revision_path])
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git show: {}", e)))?;

  if !output.status.success() {
    return Err(DominoError::Other(format!(
      "git show failed for '{}'",
      revision_path,
    )));
  }

  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ---------------------------------------------------------------------------
// Lockfile Parsing
// ---------------------------------------------------------------------------

/// Parse lockfile content into a [`LockfileData`].
///
/// `pkg_json_contents` is only used by the yarn parser (yarn.lock does not embed
/// direct dependency information, so it must be read from package.json files).
/// Pass all workspace package.json contents for complete direct dependency tracking.
pub fn parse_lockfile(
  content: &str,
  pm: &PackageManager,
  pkg_json_contents: &[String],
) -> Result<LockfileData> {
  if content.len() as u64 > MAX_LOCKFILE_BYTES {
    return Err(DominoError::Other(format!(
      "Lockfile exceeds {} MB size limit",
      MAX_LOCKFILE_BYTES / (1024 * 1024)
    )));
  }

  match pm {
    PackageManager::Npm => parse_npm_lockfile(content),
    PackageManager::Pnpm => parse_pnpm_lockfile(content),
    PackageManager::Yarn => parse_yarn_lockfile(content, pkg_json_contents),
    PackageManager::Bun => parse_bun_lockfile(content),
  }
}

fn parse_npm_lockfile(content: &str) -> Result<LockfileData> {
  let parsed: serde_json::Value = serde_json::from_str(content)
    .map_err(|e| DominoError::Parse(format!("npm lockfile: {}", e)))?;

  let mut direct_dependencies = FxHashSet::default();
  let mut packages = FxHashMap::default();

  let lockfile_version = parsed
    .get("lockfileVersion")
    .and_then(|v| v.as_u64())
    .unwrap_or(1);

  if lockfile_version >= 2 {
    if let Some(pkgs) = parsed.get("packages").and_then(|v| v.as_object()) {
      if let Some(root) = pkgs.get("") {
        for key in ["dependencies", "devDependencies", "optionalDependencies"] {
          if let Some(deps) = root.get(key).and_then(|v| v.as_object()) {
            for dep_name in deps.keys() {
              direct_dependencies.insert(dep_name.clone());
            }
          }
        }
      }

      for (key, val) in pkgs {
        if key.is_empty() {
          continue;
        }
        // Use full path as key (e.g. "node_modules/debug/node_modules/ms")
        // to avoid different instances of the same package overwriting each other.
        // Package names are extracted from keys downstream when needed.
        if extract_npm_package_name(key).is_empty() {
          continue;
        }

        let version = val
          .get("version")
          .and_then(|v| v.as_str())
          .unwrap_or("")
          .to_string();

        let mut deps = FxHashMap::default();
        for dep_key in ["dependencies", "devDependencies", "optionalDependencies"] {
          if let Some(d) = val.get(dep_key).and_then(|v| v.as_object()) {
            for (name, ver) in d {
              deps.insert(name.clone(), ver.as_str().unwrap_or("").to_string());
            }
          }
        }

        packages.insert(
          key.clone(),
          PackageInfo {
            version,
            dependencies: deps,
          },
        );
      }
    }
  } else if let Some(deps) = parsed.get("dependencies").and_then(|v| v.as_object()) {
    for dep_name in deps.keys() {
      direct_dependencies.insert(dep_name.clone());
    }
    parse_npm_v1_deps(deps, &mut packages, 0);
  }

  Ok(LockfileData {
    direct_dependencies,
    packages,
  })
}

/// Maximum nesting depth for npm v1 lockfile recursive `dependencies` objects.
const NPM_V1_MAX_DEPTH: usize = 64;

/// npm v1 uses bare package names as keys, so nested instances of the same
/// package at different versions are last-write-wins. This is acceptable for
/// the legacy format; per-path tracking would require a different data model.
fn parse_npm_v1_deps(
  deps: &serde_json::Map<String, serde_json::Value>,
  packages: &mut FxHashMap<String, PackageInfo>,
  depth: usize,
) {
  if depth > NPM_V1_MAX_DEPTH {
    return;
  }

  for (name, val) in deps {
    let version = val
      .get("version")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .to_string();

    let mut sub_deps = FxHashMap::default();
    if let Some(requires) = val.get("requires").and_then(|v| v.as_object()) {
      for (dep_name, dep_ver) in requires {
        sub_deps.insert(dep_name.clone(), dep_ver.as_str().unwrap_or("").to_string());
      }
    }

    packages.insert(
      name.clone(),
      PackageInfo {
        version,
        dependencies: sub_deps,
      },
    );

    if let Some(nested) = val.get("dependencies").and_then(|v| v.as_object()) {
      parse_npm_v1_deps(nested, packages, depth + 1);
    }
  }
}

fn extract_npm_package_name(key: &str) -> String {
  package_name_from_key(key).to_string()
}

/// Extract the bare package name from a packages-map key.
/// For npm v3 keys like `"node_modules/debug/node_modules/ms"` returns `"ms"`.
/// For other formats where the key IS the name, returns it unchanged.
/// Extract bare package name from a lockfile key.
///
/// Handles all key formats:
/// - npm v3 full paths: `node_modules/debug/node_modules/ms` → `ms`
/// - pnpm keys: `/foo@1.0.0(bar@2.0.0)` → `foo`
/// - Yarn Berry keys: `"image-lib@npm:^2.6.0"` → `image-lib`
/// - Yarn Classic keys: `lib-a@^1.0.0` → `lib-a`
/// - Bare names (bun, npm v1): `foo` → `foo`
fn package_name_from_key(key: &str) -> &str {
  // npm v3 full paths
  if let Some(last_nm) = key.rfind("node_modules/") {
    return &key[last_nm + "node_modules/".len()..];
  }

  // pnpm keys: strip leading `/`, then take name before version `@`
  let stripped = key.strip_prefix('/').unwrap_or(key);

  // Yarn Berry / Classic / pnpm: find version separator `@` (skip scope prefix)
  let search_start = if stripped.starts_with('@') { 1 } else { 0 };
  // Also strip quoted keys from Yarn Classic (e.g. `"lib-a@^1.0.0"`)
  let stripped = stripped.trim_matches('"');
  let search_start = if stripped.starts_with('@') {
    1
  } else {
    search_start
  };

  if let Some(rel_pos) = stripped[search_start..].find('@') {
    let at_pos = search_start + rel_pos;
    if at_pos > 0 {
      return &stripped[..at_pos];
    }
  }

  stripped
}

fn parse_pnpm_lockfile(content: &str) -> Result<LockfileData> {
  let parsed: serde_yaml::Value = serde_yaml::from_str(content)
    .map_err(|e| DominoError::Parse(format!("pnpm lockfile: {}", e)))?;

  let mut direct_dependencies = FxHashSet::default();
  let mut packages = FxHashMap::default();

  // Collect direct deps from ALL importers (root + workspace packages)
  if let Some(importers) = parsed.get("importers").and_then(|v| v.as_mapping()) {
    for (_importer_key, importer_val) in importers {
      if let Some(importer_map) = importer_val.as_mapping() {
        for key in ["dependencies", "devDependencies", "optionalDependencies"] {
          if let Some(deps) = importer_map
            .get(serde_yaml::Value::String(key.to_string()))
            .and_then(|v| v.as_mapping())
          {
            for (dep_name, _) in deps {
              if let Some(name) = dep_name.as_str() {
                direct_dependencies.insert(name.to_string());
              }
            }
          }
        }
      }
    }
  }

  if let Some(pkgs) = parsed.get("packages").and_then(|v| v.as_mapping()) {
    for (key, val) in pkgs {
      if let Some(key_str) = key.as_str() {
        let (pkg_name, version) = parse_pnpm_package_key(key_str);
        if pkg_name.is_empty() {
          continue;
        }

        let mut deps = FxHashMap::default();
        for dep_key in ["dependencies", "optionalDependencies"] {
          if let Some(d) = val
            .get(serde_yaml::Value::String(dep_key.to_string()))
            .and_then(|v| v.as_mapping())
          {
            for (name, ver) in d {
              if let (Some(n), Some(v)) = (name.as_str(), ver.as_str()) {
                deps.insert(n.to_string(), v.to_string());
              }
            }
          }
        }

        // Use the original lockfile key to preserve uniqueness when the same
        // package appears at multiple versions (e.g. /foo@1.0.0 and /foo@2.0.0).
        // Downstream functions use package_name_from_key() to extract bare names.
        packages.insert(
          key_str.to_string(),
          PackageInfo {
            version,
            dependencies: deps,
          },
        );
      }
    }
  }

  Ok(LockfileData {
    direct_dependencies,
    packages,
  })
}

fn parse_pnpm_package_key(key: &str) -> (String, String) {
  let key = key.strip_prefix('/').unwrap_or(key);

  // Strip pnpm v6+ parenthesized peer-dep suffix: "foo@1.0.0(bar@2.0.0)" → "foo@1.0.0"
  let key = key.split('(').next().unwrap_or(key);

  // For scoped packages "@scope/pkg@1.0.0", skip past the leading '@'.
  let search_start = if key.starts_with('@') { 1 } else { 0 };

  if let Some(rel_pos) = key[search_start..].find('@') {
    let at_pos = search_start + rel_pos;
    if at_pos == 0 {
      return (key.to_string(), String::new());
    }
    let name = &key[..at_pos];
    // Strip pnpm v5 underscore peer-dep suffix: "1.0.0_react@16.0.0" → "1.0.0"
    let version_raw = &key[at_pos + 1..];
    let version = version_raw.split('_').next().unwrap_or(version_raw);
    (name.to_string(), version.to_string())
  } else {
    (key.to_string(), String::new())
  }
}

fn parse_yarn_lockfile(content: &str, pkg_json_contents: &[String]) -> Result<LockfileData> {
  let mut direct_dependencies = FxHashSet::default();
  for pkg_json in pkg_json_contents {
    extract_direct_deps_into(pkg_json, &mut direct_dependencies);
  }

  // Yarn Berry (v2+) uses a YAML-based format with `__metadata:` header.
  // Yarn Classic (v1) uses a custom text format with `# yarn lockfile v1`.
  let packages = if content.contains("__metadata:") {
    parse_yarn_berry_packages(content)?
  } else {
    parse_yarn_classic_packages(content)?
  };

  Ok(LockfileData {
    direct_dependencies,
    packages,
  })
}

/// Extract direct dependencies from a single package.json and insert into `out`.
fn extract_direct_deps_into(pkg_json: &str, out: &mut FxHashSet<String>) {
  if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(pkg_json) {
    for key in ["dependencies", "devDependencies", "optionalDependencies"] {
      if let Some(d) = parsed.get(key).and_then(|v| v.as_object()) {
        for dep_name in d.keys() {
          out.insert(dep_name.clone());
        }
      }
    }
  }
}

/// Parse Yarn Berry (v2+/v4) lockfile.  Berry uses YAML with entries like:
/// ```text
/// "image-lib@npm:^2.6.0":
///   version: 3.3.1
///   resolution: "image-lib@npm:3.3.1"
///   dependencies:
///     sharp: "npm:^0.33.0"
///   checksum: 10c0/abc
///   languageName: node
///   linkType: hard
/// ```
fn parse_yarn_berry_packages(content: &str) -> Result<FxHashMap<String, PackageInfo>> {
  let parsed: serde_yaml::Value = serde_yaml::from_str(content)
    .map_err(|e| DominoError::Parse(format!("yarn berry lockfile: {}", e)))?;

  let mut packages = FxHashMap::default();

  let mapping = match parsed.as_mapping() {
    Some(m) => m,
    None => return Ok(packages),
  };

  for (key, val) in mapping {
    let key_str = match key.as_str() {
      Some(s) => s,
      None => continue,
    };

    // Skip __metadata and other non-package entries
    if key_str == "__metadata" || !key_str.contains('@') {
      continue;
    }

    // Validate this is a package entry
    if extract_yarn_berry_name(key_str).is_empty() {
      continue;
    }

    let entry = match val.as_mapping() {
      Some(m) => m,
      None => continue,
    };

    // serde_yaml may parse "3.3.1" as a string, but "1.0" as a float — stringify all forms.
    let version_key = serde_yaml::Value::String("version".to_string());
    let version_str = entry
      .get(&version_key)
      .map(yaml_value_to_string)
      .unwrap_or_default();

    let mut deps = FxHashMap::default();
    for dep_key in ["dependencies", "optionalDependencies", "peerDependencies"] {
      if let Some(d) = entry
        .get(serde_yaml::Value::String(dep_key.to_string()))
        .and_then(|v| v.as_mapping())
      {
        for (name, ver) in d {
          if let Some(n) = name.as_str() {
            let v = yaml_value_to_string(ver);
            let v = v.strip_prefix("npm:").unwrap_or(&v).to_string();
            deps.insert(n.to_string(), v);
          }
        }
      }
    }

    // Use the full lockfile key to preserve uniqueness when the same
    // package resolves to multiple versions from different specifiers.
    // Downstream functions use package_name_from_key() to extract bare names.
    packages.insert(
      key_str.to_string(),
      PackageInfo {
        version: version_str,
        dependencies: deps,
      },
    );
  }

  Ok(packages)
}

/// Convert a serde_yaml::Value to a trimmed string, preserving the original
/// representation (e.g. `version: 1.0` stays `"1.0"`, not `"1"`).
fn yaml_value_to_string(v: &serde_yaml::Value) -> String {
  match v {
    serde_yaml::Value::String(s) => s.clone(),
    serde_yaml::Value::Bool(b) => b.to_string(),
    _ => serde_yaml::to_string(v)
      .unwrap_or_default()
      .trim()
      .to_string(),
  }
}

/// Extract the package name from a Yarn Berry entry key.
/// Keys look like `"image-lib@npm:^2.6.0"` or `"@scope/pkg@npm:^1.0.0"`
/// or multi-specifier `"pkg@npm:^1.0.0, pkg@npm:^2.0.0"`.
fn extract_yarn_berry_name(key: &str) -> String {
  // Take the first specifier (before any comma)
  let first = key.split(',').next().unwrap_or(key).trim();

  // Find the `@npm:` or `@workspace:` protocol separator
  // For scoped packages like "@scope/pkg@npm:^1.0.0", we need the LAST `@`
  // that's followed by a protocol (npm:, workspace:, patch:, etc.)
  if let Some(pos) = first
    .rfind("@npm:")
    .or_else(|| first.rfind("@workspace:"))
    .or_else(|| first.rfind("@patch:"))
    .or_else(|| first.rfind("@file:"))
    .or_else(|| first.rfind("@link:"))
    .or_else(|| first.rfind("@portal:"))
  {
    return first[..pos].to_string();
  }

  // Fallback: find the last '@' that isn't at position 0 (scoped package prefix)
  if let Some(at_pos) = first.rfind('@') {
    if at_pos > 0 {
      return first[..at_pos].to_string();
    }
  }

  first.to_string()
}

/// Parse Yarn Classic (v1) lockfile using the text-based state machine.
fn parse_yarn_classic_packages(content: &str) -> Result<FxHashMap<String, PackageInfo>> {
  let mut packages = FxHashMap::default();

  let mut current_key: Option<String> = None;
  let mut current_version = String::new();
  let mut current_deps: FxHashMap<String, String> = FxHashMap::default();
  let mut in_dependencies = false;

  // Capture the full entry key (group 0) to preserve uniqueness when the same
  // package appears at multiple version ranges.
  static YARN_ENTRY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^("?@?[^@"\s]+@[^":\s]+"?(?:,\s*"?@?[^@"\s]+@[^":\s]+"?)*):"#)
      .expect("yarn entry regex is valid")
  });
  let entry_re = &*YARN_ENTRY_RE;

  for line in content.lines() {
    if line.starts_with('#') || line.trim().is_empty() {
      if in_dependencies {
        in_dependencies = false;
      }
      if let Some(key) = current_key.take() {
        packages.insert(
          key,
          PackageInfo {
            version: std::mem::take(&mut current_version),
            dependencies: std::mem::take(&mut current_deps),
          },
        );
      }
      continue;
    }

    if !line.starts_with(' ') && !line.starts_with('\t') {
      if let Some(key) = current_key.take() {
        packages.insert(
          key,
          PackageInfo {
            version: std::mem::take(&mut current_version),
            dependencies: std::mem::take(&mut current_deps),
          },
        );
      }
      in_dependencies = false;

      if let Some(caps) = entry_re.captures(line) {
        if let Some(m) = caps.get(1) {
          current_key = Some(m.as_str().to_string());
        }
      }
      continue;
    }

    let trimmed = line.trim();

    if trimmed.starts_with("version ") {
      current_version = trimmed
        .trim_start_matches("version ")
        .trim_matches('"')
        .to_string();
      continue;
    }

    if trimmed == "dependencies:" || trimmed == "optionalDependencies:" {
      in_dependencies = true;
      continue;
    }

    if in_dependencies {
      if let Some((dep_name, dep_ver)) = parse_yarn_classic_dep_line(trimmed) {
        current_deps.insert(dep_name, dep_ver);
      } else {
        in_dependencies = false;
      }
    }
  }

  if let Some(key) = current_key.take() {
    packages.insert(
      key,
      PackageInfo {
        version: current_version,
        dependencies: current_deps,
      },
    );
  }

  Ok(packages)
}

/// Parse a Yarn Classic dependency line like `lib-a "^1.0.0"` or `"@scope/pkg" "^1.0.0"`.
fn parse_yarn_classic_dep_line(line: &str) -> Option<(String, String)> {
  let line = line.trim();
  if line.is_empty() || line.ends_with(':') {
    return None;
  }

  if let Some(stripped) = line.strip_prefix('"') {
    let end_quote = stripped.find('"')?;
    let name = &stripped[..end_quote];
    let rest = stripped[end_quote + 1..].trim();
    let version = rest.trim_matches('"');
    return Some((name.to_string(), version.to_string()));
  }

  let parts: Vec<&str> = line.splitn(2, ' ').collect();
  if parts.len() == 2 {
    Some((
      parts[0].trim_matches('"').to_string(),
      parts[1].trim_matches('"').to_string(),
    ))
  } else {
    None
  }
}

fn parse_bun_lockfile(content: &str) -> Result<LockfileData> {
  let stripped = json_strip_comments::StripComments::new(content.as_bytes());

  let parsed: serde_json::Value = serde_json::from_reader(stripped)
    .map_err(|e| DominoError::Parse(format!("bun lockfile: {}", e)))?;

  let mut direct_dependencies = FxHashSet::default();
  let mut packages = FxHashMap::default();

  if let Some(workspaces) = parsed.get("workspaces").and_then(|v| v.as_object()) {
    if let Some(root) = workspaces.get("") {
      for key in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(deps) = root.get(key).and_then(|v| v.as_object()) {
          for dep_name in deps.keys() {
            direct_dependencies.insert(dep_name.clone());
          }
        }
      }
    }
  }

  if let Some(pkgs) = parsed.get("packages").and_then(|v| v.as_object()) {
    for (key, val) in pkgs {
      if let Some(arr) = val.as_array() {
        let version = arr
          .first()
          .and_then(|v| v.as_str())
          .unwrap_or("")
          .to_string();

        let mut deps = FxHashMap::default();
        for item in arr.iter().skip(1) {
          if let Some(obj) = item.as_object() {
            for (dep_name, dep_ver) in obj {
              if let Some(v) = dep_ver.as_str() {
                deps.insert(dep_name.clone(), v.to_string());
              }
            }
          }
        }

        packages.insert(
          key.clone(),
          PackageInfo {
            version,
            dependencies: deps,
          },
        );
      }
    }
  }

  Ok(LockfileData {
    direct_dependencies,
    packages,
  })
}

// ---------------------------------------------------------------------------
// Reverse Dependency Graph
// ---------------------------------------------------------------------------

/// Build a reverse dependency graph: for each package `P`, stores all packages
/// that list `P` as a dependency.  Uses bare package names (not full paths) so
/// the graph is consistent regardless of lockfile format.
pub fn build_reverse_dep_graph(data: &LockfileData) -> FxHashMap<&str, Vec<&str>> {
  let mut reverse: FxHashMap<&str, Vec<&str>> = FxHashMap::default();

  for (pkg_key, info) in &data.packages {
    let parent_name = package_name_from_key(pkg_key);
    for dep_name in info.dependencies.keys() {
      reverse
        .entry(dep_name.as_str())
        .or_default()
        .push(parent_name);
    }
  }

  reverse
}

/// Starting from `changed_packages`, walk the `reverse_graph` upward
/// until every changed transitive dependency is resolved to a `direct_dep`.
/// Returns only the direct dependency names that are reachable.
pub fn resolve_to_direct_deps(
  changed_packages: &FxHashSet<String>,
  reverse_graph: &FxHashMap<&str, Vec<&str>>,
  direct_deps: &FxHashSet<String>,
) -> FxHashSet<String> {
  let mut result = FxHashSet::default();

  for pkg in changed_packages {
    if direct_deps.contains(pkg) {
      result.insert(pkg.clone());
    } else {
      let mut queue = vec![pkg.as_str()];
      let mut visited = FxHashSet::default();
      visited.insert(pkg.as_str());

      while let Some(current) = queue.pop() {
        if let Some(parents) = reverse_graph.get(current) {
          for parent in parents {
            if direct_deps.contains(*parent) {
              result.insert((*parent).to_string());
            } else if visited.insert(parent) {
              queue.push(parent);
            }
          }
        }
      }
    }
  }

  result
}

// ---------------------------------------------------------------------------
// Diffing
// ---------------------------------------------------------------------------

/// Compare two sets of resolved packages and return the **package names** that
/// were added, removed, or had their version changed.
///
/// Keys in the maps may be full paths (npm v3: `node_modules/debug/node_modules/ms`)
/// or bare names (yarn, pnpm, bun: `ms`).  The returned set always contains
/// bare package names suitable for import matching.
pub fn diff_lockfile_packages(
  current: &FxHashMap<String, PackageInfo>,
  previous: &FxHashMap<String, PackageInfo>,
) -> FxHashSet<String> {
  let mut changed = FxHashSet::default();

  for (key, info) in current {
    match previous.get(key) {
      Some(prev_info) if prev_info.version != info.version => {
        changed.insert(package_name_from_key(key).to_string());
      }
      None => {
        changed.insert(package_name_from_key(key).to_string());
      }
      _ => {}
    }
  }

  for key in previous.keys() {
    if !current.contains_key(key) {
      changed.insert(package_name_from_key(key).to_string());
    }
  }

  changed
}

// ---------------------------------------------------------------------------
// High-Level API
// ---------------------------------------------------------------------------

/// Find the set of direct dependencies affected by lockfile changes.
///
/// 1. Reads the current lockfile from disk (with a size guard).
/// 2. Reads the base lockfile from git via `merge_base`.
/// 3. Parses both, diffs resolved packages, builds a reverse dependency graph,
///    and resolves transitive changes to their direct-dependency parents.
///
/// `merge_base` should be pre-computed to avoid redundant `git merge-base` calls.
pub fn find_affected_dependencies(
  cwd: &Path,
  merge_base: &str,
  pm: &PackageManager,
) -> Result<FxHashSet<String>> {
  let name = lockfile_name(pm);
  let lockfile_path = cwd.join(name);

  // Size guard: refuse to load oversized lockfiles
  let meta = fs::metadata(&lockfile_path)
    .map_err(|e| DominoError::Other(format!("Read lockfile metadata: {}", e)))?;
  if meta.len() > MAX_LOCKFILE_BYTES {
    return Err(DominoError::Other(format!(
      "Lockfile {} exceeds {} MB size limit ({} bytes)",
      name,
      MAX_LOCKFILE_BYTES / (1024 * 1024),
      meta.len(),
    )));
  }

  let current_content = fs::read_to_string(&lockfile_path)
    .map_err(|e| DominoError::Other(format!("Read lockfile: {}", e)))?;

  // For Yarn, collect package.json from root + all workspace packages.
  // Other managers embed this info in the lockfile itself.
  let current_pkg_jsons = if *pm == PackageManager::Yarn {
    collect_workspace_pkg_jsons(cwd)
  } else {
    vec![]
  };

  let base_pkg_jsons = if *pm == PackageManager::Yarn {
    collect_base_workspace_pkg_jsons(cwd, merge_base)
  } else {
    vec![]
  };

  let current_data = parse_lockfile(&current_content, pm, &current_pkg_jsons)?;

  // When the base lockfile doesn't exist (first commit, new lockfile), use an
  // empty baseline instead of trying to parse a synthetic placeholder string
  // that may not be valid for the target format (e.g. pnpm expects YAML).
  let previous_data = match get_file_from_revision(cwd, merge_base, name) {
    Ok(content) => parse_lockfile(&content, pm, &base_pkg_jsons)?,
    Err(_) => {
      debug!("Could not read base lockfile, treating all packages as new");
      LockfileData::empty()
    }
  };

  let changed_packages = diff_lockfile_packages(&current_data.packages, &previous_data.packages);
  debug!("Changed packages in lockfile: {:?}", changed_packages);

  if changed_packages.is_empty() {
    return Ok(FxHashSet::default());
  }

  // Resolve against the union of current and previous data so that removed
  // deps (absent from current) still map back to their prior importers.
  let mut merged_direct_deps = current_data.direct_dependencies.clone();
  merged_direct_deps.extend(previous_data.direct_dependencies.iter().cloned());

  let current_reverse = build_reverse_dep_graph(&current_data);
  let previous_reverse = build_reverse_dep_graph(&previous_data);

  // Merge reverse graphs: for each key in either graph, union the parent lists.
  let mut merged_reverse: FxHashMap<&str, Vec<&str>> = current_reverse;
  for (pkg, parents) in &previous_reverse {
    merged_reverse
      .entry(pkg)
      .or_default()
      .extend(parents.iter());
  }

  let affected_direct =
    resolve_to_direct_deps(&changed_packages, &merged_reverse, &merged_direct_deps);

  debug!("Affected direct dependencies: {:?}", affected_direct);

  Ok(affected_direct)
}

/// Collect package.json contents from root + workspace packages on disk.
fn collect_workspace_pkg_jsons(cwd: &Path) -> Vec<String> {
  let mut result = Vec::new();

  // Always read root package.json
  if let Ok(content) = fs::read_to_string(cwd.join("package.json")) {
    // Also discover workspace globs from root package.json
    let workspace_dirs = extract_workspace_globs(&content, cwd);
    result.push(content);

    for dir in workspace_dirs {
      let pkg_path = dir.join("package.json");
      if let Ok(pkg_content) = fs::read_to_string(&pkg_path) {
        result.push(pkg_content);
      }
    }
  }

  result
}

/// Collect package.json contents from root + workspace packages at a git revision.
fn collect_base_workspace_pkg_jsons(cwd: &Path, merge_base: &str) -> Vec<String> {
  let mut result = Vec::new();

  if let Ok(root_pkg) = get_file_from_revision(cwd, merge_base, "package.json") {
    let workspace_dirs = extract_workspace_globs(&root_pkg, cwd);
    result.push(root_pkg);

    for dir in workspace_dirs {
      let rel = dir.strip_prefix(cwd).unwrap_or(&dir).join("package.json");
      if let Ok(content) = get_file_from_revision(cwd, merge_base, &rel.to_string_lossy()) {
        result.push(content);
      }
    }
  }

  result
}

/// Parse workspace globs from package.json and expand them to directories.
fn extract_workspace_globs(pkg_json: &str, cwd: &Path) -> Vec<std::path::PathBuf> {
  let mut dirs = Vec::new();

  let parsed: serde_json::Value = match serde_json::from_str(pkg_json) {
    Ok(v) => v,
    Err(_) => return dirs,
  };

  let globs = parsed
    .get("workspaces")
    .and_then(|w| {
      // Support both array form and object form { "packages": [...] }
      w.as_array()
        .cloned()
        .or_else(|| w.get("packages").and_then(|p| p.as_array()).cloned())
    })
    .unwrap_or_default();

  for glob_val in &globs {
    if let Some(pattern) = glob_val.as_str() {
      if let Ok(entries) = glob::glob(&cwd.join(pattern).to_string_lossy()) {
        for entry in entries.flatten() {
          if entry.is_dir() {
            dirs.push(entry);
          }
        }
      }
    }
  }

  dirs
}

/// If `from_module` matches an affected direct dependency, return that
/// dependency's canonical name.  Returns `None` for relative / absolute imports
/// and for imports that don't match any affected dep.
///
/// Matches exact name (`lib-a`) or subpath (`lib-a/utils`).
pub fn match_affected_dependency<'a>(
  from_module: &str,
  affected_deps: &'a FxHashSet<String>,
) -> Option<&'a str> {
  if from_module.starts_with('.') || from_module.starts_with('/') {
    return None;
  }
  for dep in affected_deps {
    if from_module == dep.as_str() {
      return Some(dep);
    }
    if from_module.len() > dep.len()
      && from_module.starts_with(dep.as_str())
      && from_module.as_bytes()[dep.len()] == b'/'
    {
      return Some(dep);
    }
  }
  None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
  use super::*;

  // --- detect_package_manager ---

  #[test]
  fn test_detect_npm() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("package-lock.json"), "{}").unwrap();
    assert_eq!(
      detect_package_manager(tmp.path()),
      Some(PackageManager::Npm)
    );
  }

  #[test]
  fn test_detect_yarn() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("yarn.lock"), "").unwrap();
    assert_eq!(
      detect_package_manager(tmp.path()),
      Some(PackageManager::Yarn)
    );
  }

  #[test]
  fn test_detect_pnpm() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("pnpm-lock.yaml"), "").unwrap();
    assert_eq!(
      detect_package_manager(tmp.path()),
      Some(PackageManager::Pnpm)
    );
  }

  #[test]
  fn test_detect_bun() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("bun.lock"), "").unwrap();
    assert_eq!(
      detect_package_manager(tmp.path()),
      Some(PackageManager::Bun)
    );
  }

  #[test]
  fn test_detect_none() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(detect_package_manager(tmp.path()), None);
  }

  #[test]
  fn test_detect_priority_npm_over_yarn() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("package-lock.json"), "{}").unwrap();
    fs::write(tmp.path().join("yarn.lock"), "").unwrap();
    assert_eq!(
      detect_package_manager(tmp.path()),
      Some(PackageManager::Npm)
    );
  }

  // --- lockfile_name ---

  #[test]
  fn test_lockfile_name() {
    assert_eq!(lockfile_name(&PackageManager::Npm), "package-lock.json");
    assert_eq!(lockfile_name(&PackageManager::Yarn), "yarn.lock");
    assert_eq!(lockfile_name(&PackageManager::Pnpm), "pnpm-lock.yaml");
    assert_eq!(lockfile_name(&PackageManager::Bun), "bun.lock");
  }

  // --- has_lockfile_changed ---

  #[test]
  fn test_has_lockfile_changed_true() {
    let files = vec![ChangedFile {
      file_path: "package-lock.json".into(),
      changed_lines: vec![1],
    }];
    assert!(has_lockfile_changed(&files, &PackageManager::Npm));
  }

  #[test]
  fn test_has_lockfile_changed_false() {
    let files = vec![ChangedFile {
      file_path: "src/index.ts".into(),
      changed_lines: vec![1],
    }];
    assert!(!has_lockfile_changed(&files, &PackageManager::Npm));
  }

  #[test]
  fn test_has_lockfile_changed_yarn() {
    let files = vec![ChangedFile {
      file_path: "yarn.lock".into(),
      changed_lines: vec![1],
    }];
    assert!(has_lockfile_changed(&files, &PackageManager::Yarn));
  }

  #[test]
  fn test_has_lockfile_changed_pnpm() {
    let files = vec![ChangedFile {
      file_path: "pnpm-lock.yaml".into(),
      changed_lines: vec![1],
    }];
    assert!(has_lockfile_changed(&files, &PackageManager::Pnpm));
  }

  #[test]
  fn test_has_lockfile_changed_bun() {
    let files = vec![ChangedFile {
      file_path: "bun.lock".into(),
      changed_lines: vec![1],
    }];
    assert!(has_lockfile_changed(&files, &PackageManager::Bun));
  }

  // --- npm parsing ---

  #[test]
  fn test_parse_npm_lockfile_v3() {
    let content = r#"{
      "lockfileVersion": 3,
      "packages": {
        "": {
          "dependencies": { "lib-a": "^1.0.0" },
          "devDependencies": { "vitest": "^1.0.0" }
        },
        "node_modules/lib-a": {
          "version": "1.0.0",
          "dependencies": { "lib-nested-1": "^2.0.0" }
        },
        "node_modules/lib-nested-1": {
          "version": "2.0.0"
        },
        "node_modules/vitest": {
          "version": "1.0.0"
        }
      }
    }"#;

    let data = parse_npm_lockfile(content).unwrap();
    assert!(data.direct_dependencies.contains("lib-a"));
    assert!(data.direct_dependencies.contains("vitest"));
    assert_eq!(data.direct_dependencies.len(), 2);

    assert_eq!(data.packages["node_modules/lib-a"].version, "1.0.0");
    assert_eq!(
      data.packages["node_modules/lib-a"]
        .dependencies
        .get("lib-nested-1"),
      Some(&"^2.0.0".to_string())
    );
    assert_eq!(data.packages["node_modules/lib-nested-1"].version, "2.0.0");
  }

  #[test]
  fn test_parse_npm_lockfile_v1() {
    let content = r#"{
      "lockfileVersion": 1,
      "dependencies": {
        "lib-a": {
          "version": "1.0.0",
          "requires": { "lib-nested-1": "^2.0.0" },
          "dependencies": {
            "lib-nested-1": {
              "version": "2.1.0",
              "requires": { "lib-deep": "^3.0.0" }
            }
          }
        },
        "lib-nested-1": {
          "version": "2.0.0"
        }
      }
    }"#;

    let data = parse_npm_lockfile(content).unwrap();
    assert!(data.direct_dependencies.contains("lib-a"));
    assert!(data.direct_dependencies.contains("lib-nested-1"));

    assert_eq!(data.packages["lib-a"].version, "1.0.0");
    assert_eq!(
      data.packages["lib-a"].dependencies.get("lib-nested-1"),
      Some(&"^2.0.0".to_string())
    );
    // Both hoisted (2.0.0) and nested (2.1.0) versions exist; last-write-wins
    // in the flat map — the hoisted entry processes after the nested one
    assert!(data.packages.contains_key("lib-nested-1"));
  }

  #[test]
  fn test_parse_npm_lockfile_scoped_package() {
    let content = r#"{
      "lockfileVersion": 3,
      "packages": {
        "": { "dependencies": { "@scope/pkg": "^1.0.0" } },
        "node_modules/@scope/pkg": {
          "version": "1.0.0"
        }
      }
    }"#;

    let data = parse_npm_lockfile(content).unwrap();
    assert!(data.direct_dependencies.contains("@scope/pkg"));
    assert_eq!(data.packages["node_modules/@scope/pkg"].version, "1.0.0");
  }

  #[test]
  fn test_npm_v3_nested_instances_not_overwritten() {
    let base = r#"{
      "lockfileVersion": 3,
      "packages": {
        "": { "dependencies": { "ms": "^2.1.0", "debug": "^4.0.0" } },
        "node_modules/body-parser/node_modules/ms": { "version": "2.0.0" },
        "node_modules/debug": { "version": "4.3.4", "dependencies": { "ms": "2.1.2" } },
        "node_modules/debug/node_modules/ms": { "version": "2.1.3" },
        "node_modules/ms": { "version": "2.1.2" }
      }
    }"#;
    let current = r#"{
      "lockfileVersion": 3,
      "packages": {
        "": { "dependencies": { "ms": "^2.1.0", "debug": "^4.0.0" } },
        "node_modules/body-parser/node_modules/ms": { "version": "2.0.0" },
        "node_modules/debug": { "version": "4.3.4", "dependencies": { "ms": "2.1.2" } },
        "node_modules/debug/node_modules/ms": { "version": "2.1.3" },
        "node_modules/ms": { "version": "2.1.3" }
      }
    }"#;

    let base_data = parse_npm_lockfile(base).unwrap();
    let current_data = parse_npm_lockfile(current).unwrap();

    // Hoisted ms changed from 2.1.2 to 2.1.3 — must be detected
    let changed = diff_lockfile_packages(&current_data.packages, &base_data.packages);
    assert!(
      changed.contains("ms"),
      "hoisted ms version change should be detected, got: {:?}",
      changed
    );
  }

  // --- npm error cases ---

  #[test]
  fn test_parse_npm_lockfile_invalid_json() {
    let result = parse_npm_lockfile("not json at all {{{");
    assert!(result.is_err());
  }

  #[test]
  fn test_parse_npm_lockfile_empty() {
    let result = parse_npm_lockfile("");
    assert!(result.is_err());
  }

  // --- pnpm parsing ---

  #[test]
  fn test_parse_pnpm_lockfile() {
    let content = r#"
lockfileVersion: '9.0'
importers:
  '.':
    dependencies:
      lib-a:
        specifier: ^1.0.0
        version: 1.0.0
    devDependencies:
      vitest:
        specifier: ^1.0.0
        version: 1.0.0
packages:
  lib-a@1.0.0:
    dependencies:
      lib-nested-1: 2.0.0
  lib-nested-1@2.0.0: {}
  vitest@1.0.0: {}
"#;

    let data = parse_pnpm_lockfile(content).unwrap();
    assert!(data.direct_dependencies.contains("lib-a"));
    assert!(data.direct_dependencies.contains("vitest"));

    assert_eq!(data.packages["lib-a@1.0.0"].version, "1.0.0");
    assert!(data.packages["lib-a@1.0.0"]
      .dependencies
      .contains_key("lib-nested-1"));
    assert_eq!(data.packages["lib-nested-1@2.0.0"].version, "2.0.0");
  }

  #[test]
  fn test_parse_pnpm_lockfile_workspace_importers() {
    let content = r#"
lockfileVersion: '9.0'
importers:
  '.':
    dependencies:
      lib-root:
        specifier: ^1.0.0
        version: 1.0.0
  'packages/app-a':
    dependencies:
      lib-app-dep:
        specifier: ^2.0.0
        version: 2.0.0
  'packages/app-b':
    devDependencies:
      vitest:
        specifier: ^1.0.0
        version: 1.0.0
packages:
  lib-root@1.0.0: {}
  lib-app-dep@2.0.0: {}
  vitest@1.0.0: {}
"#;

    let data = parse_pnpm_lockfile(content).unwrap();
    assert!(data.direct_dependencies.contains("lib-root"));
    assert!(data.direct_dependencies.contains("lib-app-dep"));
    assert!(data.direct_dependencies.contains("vitest"));
    assert_eq!(data.direct_dependencies.len(), 3);
  }

  #[test]
  fn test_parse_pnpm_scoped_package() {
    let (name, version) = parse_pnpm_package_key("@scope/pkg@1.0.0");
    assert_eq!(name, "@scope/pkg");
    assert_eq!(version, "1.0.0");
  }

  #[test]
  fn test_parse_pnpm_peer_dep_suffix_parens() {
    let (name, version) = parse_pnpm_package_key("foo@1.0.0(bar@2.0.0)");
    assert_eq!(name, "foo");
    assert_eq!(version, "1.0.0");
  }

  #[test]
  fn test_parse_pnpm_peer_dep_suffix_underscore() {
    let (name, version) = parse_pnpm_package_key("foo@1.0.0_react@16.0.0");
    assert_eq!(name, "foo");
    assert_eq!(version, "1.0.0");
  }

  #[test]
  fn test_parse_pnpm_scoped_with_peer_suffix() {
    let (name, version) = parse_pnpm_package_key("@scope/pkg@3.2.1(peer@1.0.0)");
    assert_eq!(name, "@scope/pkg");
    assert_eq!(version, "3.2.1");
  }

  // --- pnpm error cases ---

  #[test]
  fn test_parse_pnpm_lockfile_invalid_yaml() {
    let result = parse_pnpm_lockfile("{\n\t- invalid:\n\t\t- [mixed");
    assert!(result.is_err());
  }

  // --- yarn parsing ---

  #[test]
  fn test_parse_yarn_lockfile() {
    let content = r#"# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.
# yarn lockfile v1

lib-a@^1.0.0:
  version "1.0.0"
  dependencies:
    lib-nested-1 "^2.0.0"

lib-nested-1@^2.0.0:
  version "2.0.0"
"#;

    let pkg_json = r#"{"dependencies":{"lib-a":"^1.0.0"}}"#.to_string();
    let data = parse_yarn_lockfile(content, &[pkg_json]).unwrap();
    assert!(data.direct_dependencies.contains("lib-a"));
    assert_eq!(data.packages["lib-a@^1.0.0"].version, "1.0.0");
    assert!(data.packages["lib-a@^1.0.0"]
      .dependencies
      .contains_key("lib-nested-1"));
    assert_eq!(data.packages["lib-nested-1@^2.0.0"].version, "2.0.0");
  }

  #[test]
  fn test_parse_yarn_lockfile_scoped_packages() {
    let content = r#"# yarn lockfile v1

"@scope/pkg@^1.0.0":
  version "1.0.0"
  dependencies:
    "@scope/nested" "^2.0.0"

"@scope/nested@^2.0.0":
  version "2.0.0"
"#;

    let pkg_json = r#"{"dependencies":{"@scope/pkg":"^1.0.0"}}"#.to_string();
    let data = parse_yarn_lockfile(content, &[pkg_json]).unwrap();
    assert!(data.direct_dependencies.contains("@scope/pkg"));
    assert_eq!(data.packages["\"@scope/pkg@^1.0.0\""].version, "1.0.0");
    assert!(data.packages["\"@scope/pkg@^1.0.0\""]
      .dependencies
      .contains_key("@scope/nested"));
    assert_eq!(data.packages["\"@scope/nested@^2.0.0\""].version, "2.0.0");
  }

  #[test]
  fn test_parse_yarn_lockfile_no_pkg_json() {
    let content = r#"# yarn lockfile v1

lib-a@^1.0.0:
  version "1.0.0"
"#;

    let data = parse_yarn_lockfile(content, &[]).unwrap();
    assert!(data.direct_dependencies.is_empty());
    assert_eq!(data.packages["lib-a@^1.0.0"].version, "1.0.0");
  }

  // --- yarn Berry (v2+) parsing ---

  #[test]
  fn test_parse_yarn_berry_lockfile() {
    let content = r#"__metadata:
  version: 8
  cacheKey: 10c0

"image-lib@npm:^2.6.0":
  version: 3.3.1
  resolution: "image-lib@npm:3.3.1"
  dependencies:
    sharp: "npm:^0.33.0"
    color: "npm:^4.0.0"
  checksum: 10c0/abc123
  languageName: node
  linkType: hard

"sharp@npm:^0.33.0":
  version: 0.33.5
  resolution: "sharp@npm:0.33.5"
  checksum: 10c0/def456
  languageName: node
  linkType: hard

"color@npm:^4.0.0":
  version: 4.2.3
  resolution: "color@npm:4.2.3"
  checksum: 10c0/ghi789
  languageName: node
  linkType: hard
"#;

    let pkg_json = r#"{"dependencies":{"image-lib":"^2.6.0"}}"#.to_string();
    let data = parse_yarn_lockfile(content, &[pkg_json]).unwrap();

    assert!(data.direct_dependencies.contains("image-lib"));
    assert_eq!(data.packages.len(), 3);
    assert_eq!(data.packages["image-lib@npm:^2.6.0"].version, "3.3.1");
    assert_eq!(data.packages["sharp@npm:^0.33.0"].version, "0.33.5");
    assert_eq!(data.packages["color@npm:^4.0.0"].version, "4.2.3");

    assert_eq!(
      data.packages["image-lib@npm:^2.6.0"]
        .dependencies
        .get("sharp")
        .unwrap(),
      "^0.33.0"
    );
    assert_eq!(
      data.packages["image-lib@npm:^2.6.0"]
        .dependencies
        .get("color")
        .unwrap(),
      "^4.0.0"
    );
  }

  #[test]
  fn test_parse_yarn_berry_scoped_packages() {
    let content = r#"__metadata:
  version: 8

"@scope/my-lib@npm:^1.0.0":
  version: 1.2.3
  resolution: "@scope/my-lib@npm:1.2.3"
  dependencies:
    "@scope/nested": "npm:^2.0.0"
  languageName: node
  linkType: hard

"@scope/nested@npm:^2.0.0":
  version: 2.0.1
  resolution: "@scope/nested@npm:2.0.1"
  languageName: node
  linkType: hard
"#;

    let pkg_json = r#"{"dependencies":{"@scope/my-lib":"^1.0.0"}}"#.to_string();
    let data = parse_yarn_lockfile(content, &[pkg_json]).unwrap();

    assert!(data.direct_dependencies.contains("@scope/my-lib"));
    assert_eq!(data.packages["@scope/my-lib@npm:^1.0.0"].version, "1.2.3");
    assert_eq!(data.packages["@scope/nested@npm:^2.0.0"].version, "2.0.1");
    assert!(data.packages["@scope/my-lib@npm:^1.0.0"]
      .dependencies
      .contains_key("@scope/nested"));
  }

  #[test]
  fn test_parse_yarn_berry_multi_specifier_entry() {
    let content = r#"__metadata:
  version: 8

"lodash@npm:^4.17.0, lodash@npm:^4.17.21":
  version: 4.17.21
  resolution: "lodash@npm:4.17.21"
  languageName: node
  linkType: hard
"#;

    let data = parse_yarn_lockfile(content, &[]).unwrap();
    assert_eq!(
      data.packages["lodash@npm:^4.17.0, lodash@npm:^4.17.21"].version,
      "4.17.21"
    );
  }

  #[test]
  fn test_parse_yarn_berry_no_deps() {
    let content = r#"__metadata:
  version: 8

"simple-pkg@npm:^1.0.0":
  version: 1.0.0
  resolution: "simple-pkg@npm:1.0.0"
  languageName: node
  linkType: hard
"#;

    let data = parse_yarn_lockfile(content, &[]).unwrap();
    assert_eq!(data.packages["simple-pkg@npm:^1.0.0"].version, "1.0.0");
    assert!(data.packages["simple-pkg@npm:^1.0.0"]
      .dependencies
      .is_empty());
  }

  #[test]
  fn test_yaml_value_float_preserved() {
    let val: serde_yaml::Value = serde_yaml::from_str("1.0").unwrap();
    let s = yaml_value_to_string(&val);
    assert_eq!(s, "1.0");
  }

  #[test]
  fn test_yarn_berry_version_change_detected() {
    let old_content = r#"__metadata:
  version: 8

"image-lib@npm:^2.6.0":
  version: 2.6.0
  resolution: "image-lib@npm:2.6.0"
  languageName: node
  linkType: hard
"#;
    let new_content = r#"__metadata:
  version: 8

"image-lib@npm:^2.6.0":
  version: 3.3.1
  resolution: "image-lib@npm:3.3.1"
  languageName: node
  linkType: hard
"#;

    let pkg_json = r#"{"dependencies":{"image-lib":"^2.6.0"}}"#.to_string();
    let old_data = parse_yarn_lockfile(old_content, std::slice::from_ref(&pkg_json)).unwrap();
    let new_data = parse_yarn_lockfile(new_content, std::slice::from_ref(&pkg_json)).unwrap();

    assert_eq!(old_data.packages["image-lib@npm:^2.6.0"].version, "2.6.0");
    assert_eq!(new_data.packages["image-lib@npm:^2.6.0"].version, "3.3.1");

    let changed = diff_lockfile_packages(&old_data.packages, &new_data.packages);
    assert!(
      changed.contains("image-lib"),
      "image-lib should be detected as changed"
    );
  }

  #[test]
  fn test_extract_yarn_berry_name() {
    assert_eq!(extract_yarn_berry_name("image-lib@npm:^2.6.0"), "image-lib");
    assert_eq!(
      extract_yarn_berry_name("@scope/pkg@npm:^1.0.0"),
      "@scope/pkg"
    );
    assert_eq!(
      extract_yarn_berry_name("lodash@npm:^4.17.0, lodash@npm:^4.17.21"),
      "lodash"
    );
    assert_eq!(
      extract_yarn_berry_name("@scope/pkg@workspace:*"),
      "@scope/pkg"
    );
  }

  // --- bun parsing ---

  #[test]
  fn test_parse_bun_lockfile() {
    let content = r#"{
      "lockfileVersion": 0,
      "workspaces": {
        "": {
          "name": "my-app",
          "dependencies": { "lib-a": "^1.0.0" }
        }
      },
      "packages": {
        "lib-a": ["lib-a@1.0.0", { "lib-nested-1": "^2.0.0" }],
        "lib-nested-1": ["lib-nested-1@2.0.0"]
      }
    }"#;

    let data = parse_bun_lockfile(content).unwrap();
    assert!(data.direct_dependencies.contains("lib-a"));
    assert_eq!(data.packages["lib-a"].version, "lib-a@1.0.0");
    assert!(data.packages["lib-a"]
      .dependencies
      .contains_key("lib-nested-1"));
  }

  #[test]
  fn test_parse_bun_lockfile_with_comments() {
    let content = r#"{
      // This is a JSONC comment
      "lockfileVersion": 0,
      /* block comment */
      "workspaces": {
        "": {
          "name": "my-app",
          "dependencies": { "lib-a": "^1.0.0" }
        }
      },
      "packages": {
        "lib-a": ["lib-a@1.0.0"]
      }
    }"#;

    let data = parse_bun_lockfile(content).unwrap();
    assert!(data.direct_dependencies.contains("lib-a"));
    assert_eq!(data.packages["lib-a"].version, "lib-a@1.0.0");
  }

  // --- bun error cases ---

  #[test]
  fn test_parse_bun_lockfile_invalid() {
    let result = parse_bun_lockfile("not valid jsonc {{");
    assert!(result.is_err());
  }

  // --- size guard ---

  #[test]
  fn test_parse_lockfile_rejects_oversized() {
    let result = parse_lockfile(
      &"x".repeat(MAX_LOCKFILE_BYTES as usize + 1),
      &PackageManager::Npm,
      &[],
    );
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("size limit"));
  }

  // --- reverse dep graph ---

  #[test]
  fn test_build_reverse_dep_graph() {
    let mut packages = FxHashMap::default();
    packages.insert(
      "lib-a".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: [("lib-nested-1".to_string(), "^2.0.0".to_string())]
          .into_iter()
          .collect(),
      },
    );
    packages.insert(
      "lib-b".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: [("lib-nested-1".to_string(), "^2.0.0".to_string())]
          .into_iter()
          .collect(),
      },
    );
    packages.insert(
      "lib-nested-1".to_string(),
      PackageInfo {
        version: "2.0.0".to_string(),
        dependencies: FxHashMap::default(),
      },
    );

    let data = LockfileData {
      direct_dependencies: FxHashSet::default(),
      packages,
    };

    let graph = build_reverse_dep_graph(&data);
    let parents = graph.get("lib-nested-1").unwrap();
    assert!(parents.contains(&"lib-a"));
    assert!(parents.contains(&"lib-b"));
  }

  // --- resolve_to_direct_deps ---

  #[test]
  fn test_resolve_direct_dep_changed() {
    let changed = FxHashSet::from_iter(["lib-a".to_string()]);
    let direct = FxHashSet::from_iter(["lib-a".to_string()]);
    let graph = FxHashMap::default();

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
  }

  #[test]
  fn test_resolve_transitive_dep() {
    let changed = FxHashSet::from_iter(["lib-nested-1".to_string()]);
    let direct = FxHashSet::from_iter(["lib-a".to_string()]);
    let graph = FxHashMap::from_iter([("lib-nested-1", vec!["lib-a"])]);

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
    assert!(!result.contains("lib-nested-1"));
  }

  #[test]
  fn test_resolve_multi_level_transitive() {
    let changed = FxHashSet::from_iter(["deep-lib".to_string()]);
    let direct = FxHashSet::from_iter(["lib-a".to_string()]);
    let graph = FxHashMap::from_iter([("deep-lib", vec!["mid-lib"]), ("mid-lib", vec!["lib-a"])]);

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
  }

  #[test]
  fn test_resolve_diamond_deps() {
    let changed = FxHashSet::from_iter(["deep-lib".to_string()]);
    let direct = FxHashSet::from_iter(["lib-a".to_string(), "lib-b".to_string()]);
    let graph = FxHashMap::from_iter([
      ("deep-lib", vec!["mid-a", "mid-b"]),
      ("mid-a", vec!["lib-a"]),
      ("mid-b", vec!["lib-b"]),
    ]);

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
    assert!(result.contains("lib-b"));
  }

  #[test]
  fn test_resolve_orphan_package_ignored() {
    let changed = FxHashSet::from_iter(["orphan-pkg".to_string()]);
    let direct = FxHashSet::from_iter(["lib-a".to_string()]);
    let graph = FxHashMap::default();

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.is_empty());
  }

  // --- diff ---

  #[test]
  fn test_diff_lockfile_packages_changed() {
    let current = FxHashMap::from_iter([(
      "lib-a".to_string(),
      PackageInfo {
        version: "2.0.0".to_string(),
        dependencies: FxHashMap::default(),
      },
    )]);
    let previous = FxHashMap::from_iter([(
      "lib-a".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: FxHashMap::default(),
      },
    )]);

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.contains("lib-a"));
  }

  #[test]
  fn test_diff_lockfile_packages_added() {
    let current = FxHashMap::from_iter([(
      "lib-new".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: FxHashMap::default(),
      },
    )]);
    let previous = FxHashMap::default();

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.contains("lib-new"));
  }

  #[test]
  fn test_diff_lockfile_packages_removed() {
    let current = FxHashMap::default();
    let previous = FxHashMap::from_iter([(
      "lib-old".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: FxHashMap::default(),
      },
    )]);

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.contains("lib-old"));
  }

  #[test]
  fn test_diff_lockfile_packages_unchanged() {
    let pkg = PackageInfo {
      version: "1.0.0".to_string(),
      dependencies: FxHashMap::default(),
    };
    let current = FxHashMap::from_iter([("lib-a".to_string(), pkg.clone())]);
    let previous = FxHashMap::from_iter([("lib-a".to_string(), pkg)]);

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.is_empty());
  }

  // --- match_affected_dependency ---

  #[test]
  fn test_match_affected_exact() {
    let deps = FxHashSet::from_iter(["lib-a".to_string()]);
    assert_eq!(match_affected_dependency("lib-a", &deps), Some("lib-a"));
  }

  #[test]
  fn test_match_affected_subpath() {
    let deps = FxHashSet::from_iter(["lib-a".to_string()]);
    assert_eq!(
      match_affected_dependency("lib-a/utils", &deps),
      Some("lib-a")
    );
  }

  #[test]
  fn test_match_affected_scoped() {
    let deps = FxHashSet::from_iter(["@scope/pkg".to_string()]);
    assert_eq!(
      match_affected_dependency("@scope/pkg", &deps),
      Some("@scope/pkg")
    );
    assert_eq!(
      match_affected_dependency("@scope/pkg/sub", &deps),
      Some("@scope/pkg")
    );
  }

  #[test]
  fn test_match_affected_no_match() {
    let deps = FxHashSet::from_iter(["lib-a".to_string()]);
    assert_eq!(match_affected_dependency("lib-b", &deps), None);
    assert_eq!(match_affected_dependency("./lib-a", &deps), None);
    assert_eq!(match_affected_dependency("lib-ab", &deps), None);
    assert_eq!(match_affected_dependency("/lib-a", &deps), None);
  }
}
