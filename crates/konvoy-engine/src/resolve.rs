//! Dependency graph resolution with topological ordering and cycle detection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use konvoy_config::manifest::{Manifest, PackageKind};

use crate::error::EngineError;

/// A single resolved dependency in the build graph.
#[derive(Debug, Clone)]
pub struct ResolvedDep {
    /// The dependency name (from the `[dependencies]` key).
    pub name: String,
    /// Canonical path to the dependency project root.
    pub project_root: PathBuf,
    /// Parsed manifest of the dependency.
    pub manifest: Manifest,
    /// Names of this dependency's own dependencies.
    pub dep_names: Vec<String>,
    /// SHA-256 hash of the dependency's source tree (`src/**/*.kt`).
    pub source_hash: String,
}

/// The fully resolved dependency graph in topological order.
#[derive(Debug)]
pub struct ResolvedGraph {
    /// Dependencies in topological order (leaves first, so they can be built first).
    pub order: Vec<ResolvedDep>,
}

/// Resolve all dependencies of a project into a topological build order.
///
/// # Algorithm
/// 1. For each dep in the manifest, resolve path relative to `project_root`, canonicalize.
/// 2. Read the dep's `konvoy.toml`, validate it's `kind = "lib"`.
/// 3. Recursively resolve transitive deps.
/// 4. DFS with three-color marking (white→gray→black) for cycle detection.
/// 5. Deduplicate diamond deps by canonical path.
/// 6. Enforce all deps use same Kotlin version as root.
/// 7. Return topological order (leaves first).
///
/// # Errors
/// Returns an error if a cycle is detected, a dependency is missing, a dependency
/// is not a library, or toolchain versions don't match.
pub fn resolve_dependencies(
    project_root: &Path,
    manifest: &Manifest,
) -> Result<ResolvedGraph, EngineError> {
    if manifest.dependencies.is_empty() {
        return Ok(ResolvedGraph { order: Vec::new() });
    }

    let root_kotlin = &manifest.toolchain.kotlin;

    // Collect all dependencies by canonical path to deduplicate diamonds.
    let mut visited: HashMap<PathBuf, ResolvedDep> = HashMap::new();
    // Three-color marking: 0=white, 1=gray(in-stack), 2=black(done).
    let mut color: HashMap<PathBuf, u8> = HashMap::new();
    // Topological order (post-order DFS).
    let mut topo: Vec<PathBuf> = Vec::new();

    for (dep_name, dep_spec) in &manifest.dependencies {
        let dep_path = resolve_dep_path(project_root, dep_name, dep_spec.path.as_deref())?;

        dfs(
            dep_name,
            &dep_path,
            root_kotlin,
            &mut visited,
            &mut color,
            &mut topo,
            &mut vec![manifest.package.name.clone()],
        )?;
    }

    let order = topo
        .into_iter()
        .filter_map(|path| visited.remove(&path))
        .collect();

    Ok(ResolvedGraph { order })
}

/// DFS traversal for topological sort with cycle detection.
fn dfs(
    name: &str,
    canonical_path: &Path,
    root_kotlin: &str,
    visited: &mut HashMap<PathBuf, ResolvedDep>,
    color: &mut HashMap<PathBuf, u8>,
    topo: &mut Vec<PathBuf>,
    stack: &mut Vec<String>,
) -> Result<(), EngineError> {
    let current_color = color.get(canonical_path).copied().unwrap_or(0);

    if current_color == 2 {
        // Already fully processed (black).
        return Ok(());
    }

    if current_color == 1 {
        // Gray — cycle detected.
        stack.push(name.to_owned());
        let cycle_start = stack.iter().position(|n| n == name).unwrap_or(0);
        let cycle = stack
            .get(cycle_start..)
            .unwrap_or(stack.as_slice())
            .join(" -> ");
        return Err(EngineError::DependencyCycle { cycle });
    }

    // Mark gray (in-stack).
    color.insert(canonical_path.to_path_buf(), 1);
    stack.push(name.to_owned());

    // Read the dependency manifest.
    let manifest_path = canonical_path.join("konvoy.toml");
    if !manifest_path.exists() {
        return Err(EngineError::DependencyNotFound {
            name: name.to_owned(),
            path: canonical_path.display().to_string(),
        });
    }
    let dep_manifest = Manifest::from_path(&manifest_path)?;

    // Validate: must be a library.
    if dep_manifest.package.kind != PackageKind::Lib {
        return Err(EngineError::DependencyNotLib {
            name: name.to_owned(),
            path: canonical_path.display().to_string(),
        });
    }

    // Validate: same Kotlin version.
    if dep_manifest.toolchain.kotlin != root_kotlin {
        return Err(EngineError::DependencyToolchainMismatch {
            name: name.to_owned(),
            dep_version: dep_manifest.toolchain.kotlin.clone(),
            root_version: root_kotlin.to_owned(),
        });
    }

    // Recurse into this dep's own dependencies.
    let dep_names: Vec<String> = dep_manifest.dependencies.keys().cloned().collect();
    for (sub_name, sub_spec) in &dep_manifest.dependencies {
        let sub_path = resolve_dep_path(canonical_path, sub_name, sub_spec.path.as_deref())?;
        dfs(
            sub_name,
            &sub_path,
            root_kotlin,
            visited,
            color,
            topo,
            stack,
        )?;
    }

    // Mark black (done) and add to topo order.
    color.insert(canonical_path.to_path_buf(), 2);
    stack.pop();

    // Compute source hash for integrity verification.
    let src_dir = canonical_path.join("src");
    let source_hash = konvoy_util::hash::sha256_dir(&src_dir, "**/*.kt").unwrap_or_default();

    visited.insert(
        canonical_path.to_path_buf(),
        ResolvedDep {
            name: name.to_owned(),
            project_root: canonical_path.to_path_buf(),
            manifest: dep_manifest,
            dep_names,
            source_hash,
        },
    );
    topo.push(canonical_path.to_path_buf());

    Ok(())
}

/// Maximum number of `..` components allowed above the parent root.
///
/// This allows sibling dependencies (e.g. `../my-lib`) and reasonable workspace
/// layouts while blocking deeply nested traversals that escape the project tree.
const MAX_PARENT_TRAVERSAL: usize = 5;

/// Resolve a dependency path relative to the parent project root.
fn resolve_dep_path(
    parent_root: &Path,
    dep_name: &str,
    path: Option<&str>,
) -> Result<PathBuf, EngineError> {
    let Some(rel_path) = path else {
        return Err(EngineError::DependencyNotFound {
            name: dep_name.to_owned(),
            path: "<no path specified>".to_owned(),
        });
    };

    // Reject absolute paths — dependencies must be relative to the project.
    if Path::new(rel_path).is_absolute() {
        return Err(EngineError::DependencyPathEscape {
            name: dep_name.to_owned(),
            path: rel_path.to_owned(),
        });
    }

    // Count how many leading `..` components escape above parent_root.
    let parent_escapes = Path::new(rel_path)
        .components()
        .take_while(|c| matches!(c, std::path::Component::ParentDir))
        .count();
    if parent_escapes > MAX_PARENT_TRAVERSAL {
        return Err(EngineError::DependencyPathEscape {
            name: dep_name.to_owned(),
            path: parent_root.join(rel_path).display().to_string(),
        });
    }

    let resolved = parent_root.join(rel_path);
    resolved
        .canonicalize()
        .map_err(|_| EngineError::DependencyNotFound {
            name: dep_name.to_owned(),
            path: resolved.display().to_string(),
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;

    fn write_manifest(dir: &Path, name: &str, kind: &str, deps: &str) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.kt"), "// lib").unwrap();
        let kind_line = if kind == "bin" {
            String::new()
        } else {
            format!("kind = \"{kind}\"\n")
        };
        let deps_section = if deps.is_empty() {
            String::new()
        } else {
            format!("\n[dependencies]\n{deps}")
        };
        fs::write(
            dir.join("konvoy.toml"),
            format!(
                "[package]\nname = \"{name}\"\n{kind_line}\n[toolchain]\nkotlin = \"2.1.0\"\n{deps_section}"
            ),
        )
        .unwrap();
    }

    #[test]
    fn no_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), "root", "bin", "");
        let manifest = Manifest::from_path(&tmp.path().join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(tmp.path(), &manifest).unwrap();
        assert!(graph.order.is_empty());
    }

    #[test]
    fn single_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let lib_dir = tmp.path().join("my-lib");
        write_manifest(&lib_dir, "my-lib", "lib", "");

        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "my-lib = { path = \"../my-lib\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        assert_eq!(graph.order.len(), 1);
        assert_eq!(graph.order.first().unwrap().name, "my-lib");
    }

    #[test]
    fn transitive_dependencies() {
        let tmp = tempfile::tempdir().unwrap();

        // leaf has no deps
        let leaf_dir = tmp.path().join("leaf");
        write_manifest(&leaf_dir, "leaf", "lib", "");

        // mid depends on leaf
        let mid_dir = tmp.path().join("mid");
        write_manifest(&mid_dir, "mid", "lib", "leaf = { path = \"../leaf\" }\n");

        // root depends on mid
        let root_dir = tmp.path().join("root");
        write_manifest(&root_dir, "root", "bin", "mid = { path = \"../mid\" }\n");

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        assert_eq!(graph.order.len(), 2);
        // leaf must come before mid (topo order)
        assert_eq!(graph.order.first().unwrap().name, "leaf");
        assert_eq!(graph.order.get(1).unwrap().name, "mid");
    }

    #[test]
    fn diamond_deduplication() {
        let tmp = tempfile::tempdir().unwrap();

        // shared: no deps
        let shared_dir = tmp.path().join("shared");
        write_manifest(&shared_dir, "shared", "lib", "");

        // a depends on shared
        let a_dir = tmp.path().join("a");
        write_manifest(&a_dir, "a", "lib", "shared = { path = \"../shared\" }\n");

        // b depends on shared
        let b_dir = tmp.path().join("b");
        write_manifest(&b_dir, "b", "lib", "shared = { path = \"../shared\" }\n");

        // root depends on a and b
        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "a = { path = \"../a\" }\nb = { path = \"../b\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        // shared should only appear once
        let shared_count = graph.order.iter().filter(|d| d.name == "shared").count();
        assert_eq!(shared_count, 1);
        assert_eq!(graph.order.len(), 3); // shared, a, b
    }

    #[test]
    fn cycle_detection() {
        let tmp = tempfile::tempdir().unwrap();

        // a depends on b
        let a_dir = tmp.path().join("a");
        write_manifest(&a_dir, "a", "lib", "b = { path = \"../b\" }\n");

        // b depends on a (cycle!)
        let b_dir = tmp.path().join("b");
        write_manifest(&b_dir, "b", "lib", "a = { path = \"../a\" }\n");

        // root depends on a
        let root_dir = tmp.path().join("root");
        write_manifest(&root_dir, "root", "bin", "a = { path = \"../a\" }\n");

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let result = resolve_dependencies(&root_dir, &manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cycle"), "error was: {err}");
    }

    #[test]
    fn missing_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "missing = { path = \"../nonexistent\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let result = resolve_dependencies(&root_dir, &manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"), "error was: {err}");
    }

    #[test]
    fn non_lib_dependency() {
        let tmp = tempfile::tempdir().unwrap();

        // dep is a bin, not a lib
        let dep_dir = tmp.path().join("dep");
        write_manifest(&dep_dir, "dep", "bin", "");

        let root_dir = tmp.path().join("root");
        write_manifest(&root_dir, "root", "bin", "dep = { path = \"../dep\" }\n");

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let result = resolve_dependencies(&root_dir, &manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("lib"), "error was: {err}");
    }

    #[test]
    fn source_hash_computed_for_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let lib_dir = tmp.path().join("my-lib");
        write_manifest(&lib_dir, "my-lib", "lib", "");

        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "my-lib = { path = \"../my-lib\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        assert_eq!(graph.order.len(), 1);
        let dep = graph.order.first().unwrap();
        assert!(
            !dep.source_hash.is_empty(),
            "source_hash should be computed"
        );
    }

    #[test]
    fn source_hash_changes_when_source_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let lib_dir = tmp.path().join("my-lib");
        write_manifest(&lib_dir, "my-lib", "lib", "");

        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "my-lib = { path = \"../my-lib\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph1 = resolve_dependencies(&root_dir, &manifest).unwrap();
        let hash1 = graph1.order.first().unwrap().source_hash.clone();

        // Modify the dependency source.
        fs::write(lib_dir.join("src/lib.kt"), "// modified").unwrap();

        let graph2 = resolve_dependencies(&root_dir, &manifest).unwrap();
        let hash2 = graph2.order.first().unwrap().source_hash.clone();

        assert_ne!(
            hash1, hash2,
            "source_hash should change when source changes"
        );
    }

    #[test]
    fn toolchain_mismatch() {
        let tmp = tempfile::tempdir().unwrap();

        // dep uses different Kotlin version
        let dep_dir = tmp.path().join("dep");
        fs::create_dir_all(dep_dir.join("src")).unwrap();
        fs::write(dep_dir.join("src/lib.kt"), "// lib").unwrap();
        fs::write(
            dep_dir.join("konvoy.toml"),
            "[package]\nname = \"dep\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"2.0.0\"\n",
        )
        .unwrap();

        let root_dir = tmp.path().join("root");
        write_manifest(&root_dir, "root", "bin", "dep = { path = \"../dep\" }\n");

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let result = resolve_dependencies(&root_dir, &manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("2.0.0"), "error was: {err}");
        assert!(err.contains("2.1.0"), "error was: {err}");
    }

    #[test]
    fn sibling_dependency_allowed() {
        // ../sibling-lib is a common pattern and must work
        let tmp = tempfile::tempdir().unwrap();
        let lib_dir = tmp.path().join("sibling-lib");
        write_manifest(&lib_dir, "sibling-lib", "lib", "");

        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "sibling-lib = { path = \"../sibling-lib\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        assert_eq!(graph.order.len(), 1);
        assert_eq!(graph.order.first().unwrap().name, "sibling-lib");
    }

    #[test]
    fn deep_traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "evil = { path = \"../../../../../../../../../../etc/passwd\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let result = resolve_dependencies(&root_dir, &manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("escapes the project tree"), "error was: {err}");
    }

    #[test]
    fn absolute_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "evil = { path = \"/etc/something\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let result = resolve_dependencies(&root_dir, &manifest);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("escapes the project tree"), "error was: {err}");
    }
}
