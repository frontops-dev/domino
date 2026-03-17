use crate::error::{DominoError, Result};
use crate::types::ChangedFile;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use tracing::debug;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageManager {
  Npm,
  Yarn,
  Pnpm,
  Bun,
}

#[derive(Debug, Clone)]
pub struct PackageInfo {
  pub version: String,
  pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct LockfileData {
  pub direct_dependencies: HashSet<String>,
  pub packages: HashMap<String, PackageInfo>,
}

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

pub fn lockfile_name(pm: &PackageManager) -> &'static str {
  match pm {
    PackageManager::Npm => "package-lock.json",
    PackageManager::Yarn => "yarn.lock",
    PackageManager::Pnpm => "pnpm-lock.yaml",
    PackageManager::Bun => "bun.lock",
  }
}

pub fn has_lockfile_changed(changed_files: &[ChangedFile], pm: &PackageManager) -> bool {
  let name = lockfile_name(pm);
  changed_files
    .iter()
    .any(|f| f.file_path.to_str() == Some(name))
}

pub fn get_file_from_revision(repo_path: &Path, base: &str, file_path: &str) -> Result<String> {
  let revision_path = format!("{}:{}", base, file_path);
  let output = Command::new("git")
    .args(["show", &revision_path])
    .current_dir(repo_path)
    .output()
    .map_err(|e| DominoError::Other(format!("Failed to execute git show: {}", e)))?;

  if !output.status.success() {
    return Err(DominoError::Other(format!(
      "git show failed for '{}': {}",
      revision_path,
      String::from_utf8_lossy(&output.stderr)
    )));
  }

  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// --- Lockfile Parsing ---

pub fn parse_lockfile(content: &str, pm: &PackageManager, cwd: &Path) -> Result<LockfileData> {
  match pm {
    PackageManager::Npm => parse_npm_lockfile(content),
    PackageManager::Pnpm => parse_pnpm_lockfile(content),
    PackageManager::Yarn => parse_yarn_lockfile(content, cwd),
    PackageManager::Bun => parse_bun_lockfile(content),
  }
}

fn parse_npm_lockfile(content: &str) -> Result<LockfileData> {
  let parsed: serde_json::Value = serde_json::from_str(content)
    .map_err(|e| DominoError::Parse(format!("npm lockfile: {}", e)))?;

  let mut direct_dependencies = HashSet::new();
  let mut packages = HashMap::new();

  let lockfile_version = parsed
    .get("lockfileVersion")
    .and_then(|v| v.as_u64())
    .unwrap_or(1);

  if lockfile_version >= 2 {
    if let Some(pkgs) = parsed.get("packages").and_then(|v| v.as_object()) {
      // Root entry "" has direct deps
      if let Some(root) = pkgs.get("") {
        for key in ["dependencies", "devDependencies"] {
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
        let pkg_name = extract_npm_package_name(key);
        if pkg_name.is_empty() {
          continue;
        }

        let version = val
          .get("version")
          .and_then(|v| v.as_str())
          .unwrap_or("")
          .to_string();

        let mut deps = HashMap::new();
        for dep_key in ["dependencies", "devDependencies", "optionalDependencies"] {
          if let Some(d) = val.get(dep_key).and_then(|v| v.as_object()) {
            for (name, ver) in d {
              deps.insert(name.clone(), ver.as_str().unwrap_or("").to_string());
            }
          }
        }

        packages.insert(
          pkg_name,
          PackageInfo {
            version,
            dependencies: deps,
          },
        );
      }
    }
  } else {
    // v1: uses "dependencies" at root level
    if let Some(deps) = parsed.get("dependencies").and_then(|v| v.as_object()) {
      for dep_name in deps.keys() {
        direct_dependencies.insert(dep_name.clone());
      }
      parse_npm_v1_deps(deps, &mut packages);
    }
  }

  Ok(LockfileData {
    direct_dependencies,
    packages,
  })
}

fn parse_npm_v1_deps(
  deps: &serde_json::Map<String, serde_json::Value>,
  packages: &mut HashMap<String, PackageInfo>,
) {
  for (name, val) in deps {
    let version = val
      .get("version")
      .and_then(|v| v.as_str())
      .unwrap_or("")
      .to_string();

    let mut sub_deps = HashMap::new();
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
      parse_npm_v1_deps(nested, packages);
    }
  }
}

fn extract_npm_package_name(key: &str) -> String {
  // Keys look like "node_modules/lodash" or "node_modules/@scope/pkg"
  // or nested "node_modules/a/node_modules/b"
  // We want the last package name in the chain
  if let Some(last_nm) = key.rfind("node_modules/") {
    let after = &key[last_nm + "node_modules/".len()..];
    after.to_string()
  } else {
    key.to_string()
  }
}

fn parse_pnpm_lockfile(content: &str) -> Result<LockfileData> {
  let parsed: serde_yaml::Value = serde_yaml::from_str(content)
    .map_err(|e| DominoError::Parse(format!("pnpm lockfile: {}", e)))?;

  let mut direct_dependencies = HashSet::new();
  let mut packages = HashMap::new();

  // Direct deps from importers['.'].dependencies + devDependencies
  if let Some(importers) = parsed.get("importers").and_then(|v| v.as_mapping()) {
    if let Some(root) = importers
      .get(serde_yaml::Value::String(".".to_string()))
      .and_then(|v| v.as_mapping())
    {
      for key in ["dependencies", "devDependencies"] {
        if let Some(deps) = root
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

  // All packages
  if let Some(pkgs) = parsed.get("packages").and_then(|v| v.as_mapping()) {
    for (key, val) in pkgs {
      if let Some(key_str) = key.as_str() {
        let (pkg_name, version) = parse_pnpm_package_key(key_str);
        if pkg_name.is_empty() {
          continue;
        }

        let mut deps = HashMap::new();
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

        packages.insert(
          pkg_name,
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
  // Keys look like "lib-a@1.0.0" or "@scope/pkg@1.0.0"
  // For scoped packages, we need to find the last '@' that starts the version
  let key = key.strip_prefix('/').unwrap_or(key);

  if let Some(at_pos) = key.rfind('@') {
    if at_pos == 0 {
      // Only one '@', this is just "@scope/pkg" without version
      return (key.to_string(), String::new());
    }
    let name = &key[..at_pos];
    let version = &key[at_pos + 1..];
    (name.to_string(), version.to_string())
  } else {
    (key.to_string(), String::new())
  }
}

fn parse_yarn_lockfile(content: &str, cwd: &Path) -> Result<LockfileData> {
  let mut direct_dependencies = HashSet::new();
  let mut packages = HashMap::new();

  // Yarn.lock doesn't track direct deps; read from package.json
  let pkg_json_path = cwd.join("package.json");
  if let Ok(pkg_content) = fs::read_to_string(&pkg_json_path) {
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&pkg_content) {
      for key in ["dependencies", "devDependencies"] {
        if let Some(deps) = parsed.get(key).and_then(|v| v.as_object()) {
          for dep_name in deps.keys() {
            direct_dependencies.insert(dep_name.clone());
          }
        }
      }
    }
  }

  // Parse yarn.lock: state machine
  let mut current_name: Option<String> = None;
  let mut current_version = String::new();
  let mut current_deps: HashMap<String, String> = HashMap::new();
  let mut in_dependencies = false;

  let entry_re = Regex::new(r#"^"?(@?[^@"\s]+)@[^":\s]+"?(?:,\s*"?@?[^@"\s]+@[^":\s]+"?)*:"#)
    .map_err(|e| DominoError::Parse(format!("yarn regex: {}", e)))?;

  for line in content.lines() {
    if line.starts_with('#') || line.trim().is_empty() {
      if in_dependencies {
        in_dependencies = false;
      }
      if let Some(name) = current_name.take() {
        packages.insert(
          name,
          PackageInfo {
            version: std::mem::take(&mut current_version),
            dependencies: std::mem::take(&mut current_deps),
          },
        );
      }
      continue;
    }

    // New entry header
    if !line.starts_with(' ') && !line.starts_with('\t') {
      // Flush previous entry
      if let Some(name) = current_name.take() {
        packages.insert(
          name,
          PackageInfo {
            version: std::mem::take(&mut current_version),
            dependencies: std::mem::take(&mut current_deps),
          },
        );
      }
      in_dependencies = false;

      if let Some(caps) = entry_re.captures(line) {
        if let Some(m) = caps.get(1) {
          current_name = Some(m.as_str().to_string());
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
      // Lines like: "    lib-nested-1 "^2.0.0""
      if let Some((dep_name, dep_ver)) = parse_yarn_dep_line(trimmed) {
        current_deps.insert(dep_name, dep_ver);
      } else {
        in_dependencies = false;
      }
    }
  }

  // Flush last entry
  if let Some(name) = current_name.take() {
    packages.insert(
      name,
      PackageInfo {
        version: current_version,
        dependencies: current_deps,
      },
    );
  }

  Ok(LockfileData {
    direct_dependencies,
    packages,
  })
}

fn parse_yarn_dep_line(line: &str) -> Option<(String, String)> {
  // Lines like: `lib-nested-1 "^2.0.0"` or `"@scope/pkg" "^1.0.0"`
  let line = line.trim();
  if line.is_empty() || line.ends_with(':') {
    return None;
  }

  let parts: Vec<&str> = if let Some(stripped) = line.strip_prefix('"') {
    // Scoped package: "@scope/pkg" "^1.0.0"
    let end_quote = stripped.find('"')?;
    let name = &stripped[..end_quote];
    let rest = stripped[end_quote + 1..].trim();
    let version = rest.trim_matches('"');
    return Some((name.to_string(), version.to_string()));
  } else {
    line.splitn(2, ' ').collect()
  };

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
  // bun.lock is JSONC (JSON with comments) - strip comments before parsing
  let stripped = json_strip_comments::StripComments::new(content.as_bytes());

  let parsed: serde_json::Value = serde_json::from_reader(stripped)
    .map_err(|e| DominoError::Parse(format!("bun lockfile: {}", e)))?;

  let mut direct_dependencies = HashSet::new();
  let mut packages = HashMap::new();

  // Direct deps from workspaces[""] or workspaces[""]
  if let Some(workspaces) = parsed.get("workspaces").and_then(|v| v.as_object()) {
    if let Some(root) = workspaces.get("") {
      for key in ["dependencies", "devDependencies"] {
        if let Some(deps) = root.get(key).and_then(|v| v.as_object()) {
          for dep_name in deps.keys() {
            direct_dependencies.insert(dep_name.clone());
          }
        }
      }
    }
  }

  // Packages - bun.lock format uses arrays for package entries
  if let Some(pkgs) = parsed.get("packages").and_then(|v| v.as_object()) {
    for (key, val) in pkgs {
      if let Some(arr) = val.as_array() {
        // First element is typically "pkg@version" or version info
        let version = arr
          .first()
          .and_then(|v| v.as_str())
          .unwrap_or("")
          .to_string();

        let mut deps = HashMap::new();
        // Dependencies may be in a later array element as an object
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

// --- Reverse Dependency Graph ---

pub fn build_reverse_dep_graph(data: &LockfileData) -> HashMap<String, Vec<String>> {
  let mut reverse: HashMap<String, Vec<String>> = HashMap::new();

  for (pkg_name, info) in &data.packages {
    for dep_name in info.dependencies.keys() {
      reverse
        .entry(dep_name.clone())
        .or_default()
        .push(pkg_name.clone());
    }
  }

  reverse
}

pub fn resolve_to_direct_deps(
  changed_packages: &HashSet<String>,
  reverse_graph: &HashMap<String, Vec<String>>,
  direct_deps: &HashSet<String>,
) -> HashSet<String> {
  let mut result = HashSet::new();

  for pkg in changed_packages {
    if direct_deps.contains(pkg) {
      result.insert(pkg.clone());
    } else {
      // BFS up the reverse graph until we reach direct deps
      let mut queue = vec![pkg.as_str()];
      let mut visited = HashSet::new();
      visited.insert(pkg.as_str());

      while let Some(current) = queue.pop() {
        if let Some(parents) = reverse_graph.get(current) {
          for parent in parents {
            if direct_deps.contains(parent) {
              result.insert(parent.clone());
            } else if visited.insert(parent.as_str()) {
              queue.push(parent.as_str());
            }
          }
        }
      }
    }
  }

  result
}

// --- Diffing ---

pub fn diff_lockfile_packages(
  current: &HashMap<String, PackageInfo>,
  previous: &HashMap<String, PackageInfo>,
) -> HashSet<String> {
  let mut changed = HashSet::new();

  for (name, info) in current {
    match previous.get(name) {
      Some(prev_info) if prev_info.version != info.version => {
        changed.insert(name.clone());
      }
      None => {
        changed.insert(name.clone());
      }
      _ => {}
    }
  }

  // Removed packages
  for name in previous.keys() {
    if !current.contains_key(name) {
      changed.insert(name.clone());
    }
  }

  changed
}

// --- High-Level API ---

/// Find the set of direct dependencies affected by lockfile changes.
/// This parses both current and base lockfiles, diffs them, builds a reverse
/// dependency graph, and resolves transitive changes to their direct-dep parents.
pub fn find_affected_dependencies(
  cwd: &Path,
  base: &str,
  pm: &PackageManager,
) -> Result<HashSet<String>> {
  let name = lockfile_name(pm);

  // Read current lockfile
  let current_content = fs::read_to_string(cwd.join(name))
    .map_err(|e| DominoError::Other(format!("Read lockfile: {}", e)))?;

  // Read base lockfile from git
  let merge_base = crate::git::get_merge_base(cwd, base, "HEAD")?;
  let previous_content = match get_file_from_revision(cwd, &merge_base, name) {
    Ok(content) => content,
    Err(_) => {
      debug!("Could not read base lockfile, treating all packages as new");
      "{}".to_string()
    }
  };

  let current_data = parse_lockfile(&current_content, pm, cwd)?;
  let previous_data = parse_lockfile(&previous_content, pm, cwd)?;

  let changed_packages = diff_lockfile_packages(&current_data.packages, &previous_data.packages);
  debug!("Changed packages in lockfile: {:?}", changed_packages);

  if changed_packages.is_empty() {
    return Ok(HashSet::new());
  }

  let reverse_graph = build_reverse_dep_graph(&current_data);
  let affected_direct = resolve_to_direct_deps(
    &changed_packages,
    &reverse_graph,
    &current_data.direct_dependencies,
  );

  debug!("Affected direct dependencies: {:?}", affected_direct);

  Ok(affected_direct)
}

/// Check if an import's `from_module` matches any affected direct dependency.
pub fn is_affected_import(from_module: &str, affected_deps: &HashSet<String>) -> bool {
  for dep in affected_deps {
    if from_module == dep || from_module.starts_with(&format!("{}/", dep)) {
      return true;
    }
  }
  false
}

#[cfg(test)]
mod tests {
  use super::*;

  // --- detect_package_manager ---

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

    assert_eq!(data.packages["lib-a"].version, "1.0.0");
    assert_eq!(
      data.packages["lib-a"].dependencies.get("lib-nested-1"),
      Some(&"^2.0.0".to_string())
    );
    assert_eq!(data.packages["lib-nested-1"].version, "2.0.0");
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
    assert_eq!(data.packages["@scope/pkg"].version, "1.0.0");
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

    assert_eq!(data.packages["lib-a"].version, "1.0.0");
    assert!(data.packages["lib-a"]
      .dependencies
      .contains_key("lib-nested-1"));
    assert_eq!(data.packages["lib-nested-1"].version, "2.0.0");
  }

  #[test]
  fn test_parse_pnpm_scoped_package() {
    let (name, version) = parse_pnpm_package_key("@scope/pkg@1.0.0");
    assert_eq!(name, "@scope/pkg");
    assert_eq!(version, "1.0.0");
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

    // Yarn needs a package.json for direct deps; use a tempdir
    let tmp = tempfile::tempdir().unwrap();
    fs::write(
      tmp.path().join("package.json"),
      r#"{"dependencies":{"lib-a":"^1.0.0"}}"#,
    )
    .unwrap();

    let data = parse_yarn_lockfile(content, tmp.path()).unwrap();
    assert!(data.direct_dependencies.contains("lib-a"));
    assert_eq!(data.packages["lib-a"].version, "1.0.0");
    assert!(data.packages["lib-a"]
      .dependencies
      .contains_key("lib-nested-1"));
    assert_eq!(data.packages["lib-nested-1"].version, "2.0.0");
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

  // --- reverse dep graph ---

  #[test]
  fn test_build_reverse_dep_graph() {
    let mut packages = HashMap::new();
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
        dependencies: HashMap::new(),
      },
    );

    let data = LockfileData {
      direct_dependencies: HashSet::new(),
      packages,
    };

    let graph = build_reverse_dep_graph(&data);
    let parents = graph.get("lib-nested-1").unwrap();
    assert!(parents.contains(&"lib-a".to_string()));
    assert!(parents.contains(&"lib-b".to_string()));
  }

  // --- resolve_to_direct_deps ---

  #[test]
  fn test_resolve_direct_dep_changed() {
    let changed = HashSet::from(["lib-a".to_string()]);
    let direct = HashSet::from(["lib-a".to_string()]);
    let graph = HashMap::new();

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
  }

  #[test]
  fn test_resolve_transitive_dep() {
    let changed = HashSet::from(["lib-nested-1".to_string()]);
    let direct = HashSet::from(["lib-a".to_string()]);
    let graph = HashMap::from([("lib-nested-1".to_string(), vec!["lib-a".to_string()])]);

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
    assert!(!result.contains("lib-nested-1"));
  }

  #[test]
  fn test_resolve_multi_level_transitive() {
    let changed = HashSet::from(["deep-lib".to_string()]);
    let direct = HashSet::from(["lib-a".to_string()]);
    let graph = HashMap::from([
      ("deep-lib".to_string(), vec!["mid-lib".to_string()]),
      ("mid-lib".to_string(), vec!["lib-a".to_string()]),
    ]);

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
  }

  #[test]
  fn test_resolve_diamond_deps() {
    // deep-lib -> mid-a -> lib-a (direct)
    // deep-lib -> mid-b -> lib-b (direct)
    let changed = HashSet::from(["deep-lib".to_string()]);
    let direct = HashSet::from(["lib-a".to_string(), "lib-b".to_string()]);
    let graph = HashMap::from([
      (
        "deep-lib".to_string(),
        vec!["mid-a".to_string(), "mid-b".to_string()],
      ),
      ("mid-a".to_string(), vec!["lib-a".to_string()]),
      ("mid-b".to_string(), vec!["lib-b".to_string()]),
    ]);

    let result = resolve_to_direct_deps(&changed, &graph, &direct);
    assert!(result.contains("lib-a"));
    assert!(result.contains("lib-b"));
  }

  // --- diff ---

  #[test]
  fn test_diff_lockfile_packages_changed() {
    let current = HashMap::from([(
      "lib-a".to_string(),
      PackageInfo {
        version: "2.0.0".to_string(),
        dependencies: HashMap::new(),
      },
    )]);
    let previous = HashMap::from([(
      "lib-a".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: HashMap::new(),
      },
    )]);

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.contains("lib-a"));
  }

  #[test]
  fn test_diff_lockfile_packages_added() {
    let current = HashMap::from([(
      "lib-new".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: HashMap::new(),
      },
    )]);
    let previous = HashMap::new();

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.contains("lib-new"));
  }

  #[test]
  fn test_diff_lockfile_packages_removed() {
    let current = HashMap::new();
    let previous = HashMap::from([(
      "lib-old".to_string(),
      PackageInfo {
        version: "1.0.0".to_string(),
        dependencies: HashMap::new(),
      },
    )]);

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.contains("lib-old"));
  }

  #[test]
  fn test_diff_lockfile_packages_unchanged() {
    let pkg = PackageInfo {
      version: "1.0.0".to_string(),
      dependencies: HashMap::new(),
    };
    let current = HashMap::from([("lib-a".to_string(), pkg.clone())]);
    let previous = HashMap::from([("lib-a".to_string(), pkg)]);

    let result = diff_lockfile_packages(&current, &previous);
    assert!(result.is_empty());
  }

  // --- is_affected_import ---

  #[test]
  fn test_is_affected_import_exact() {
    let deps = HashSet::from(["lib-a".to_string()]);
    assert!(is_affected_import("lib-a", &deps));
  }

  #[test]
  fn test_is_affected_import_subpath() {
    let deps = HashSet::from(["lib-a".to_string()]);
    assert!(is_affected_import("lib-a/utils", &deps));
  }

  #[test]
  fn test_is_affected_import_scoped() {
    let deps = HashSet::from(["@scope/pkg".to_string()]);
    assert!(is_affected_import("@scope/pkg", &deps));
    assert!(is_affected_import("@scope/pkg/sub", &deps));
  }

  #[test]
  fn test_is_affected_import_no_match() {
    let deps = HashSet::from(["lib-a".to_string()]);
    assert!(!is_affected_import("lib-b", &deps));
    assert!(!is_affected_import("./lib-a", &deps));
    assert!(!is_affected_import("lib-ab", &deps));
  }
}
