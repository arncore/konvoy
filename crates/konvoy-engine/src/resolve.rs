//! Dependency graph resolution with topological ordering and cycle detection.

use std::collections::{HashMap, HashSet};
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

/// Group dependencies into parallel build levels.
///
/// Each level contains deps whose own dependencies are all in previous levels.
/// Deps within the same level can be built concurrently.
pub fn parallel_levels(graph: &ResolvedGraph) -> Vec<Vec<&ResolvedDep>> {
    let mut levels: Vec<Vec<&ResolvedDep>> = Vec::new();
    let mut assigned: HashSet<&str> = HashSet::new();
    let mut remaining: Vec<&ResolvedDep> = graph.order.iter().collect();

    while !remaining.is_empty() {
        let (level, rest): (Vec<_>, Vec<_>) = remaining
            .into_iter()
            .partition(|dep| dep.dep_names.iter().all(|d| assigned.contains(d.as_str())));

        for dep in &level {
            assigned.insert(&dep.name);
        }
        levels.push(level);
        remaining = rest;
    }

    levels
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
    let source_hash = konvoy_util::hash::sha256_dir(&src_dir, "**/*.kt")?;

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
const MAX_PARENT_TRAVERSAL: usize = 3;

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
    fn three_parent_traversals_accepted() {
        // Exactly MAX_PARENT_TRAVERSAL (3) leading `..` components should be allowed.
        // Layout:
        //   tmp/ws/lib/           <- the dependency library
        //   tmp/ws/a/b/c/root/    <- the project root
        // From root, `../../../lib` traverses 3 parent dirs (c -> b -> a -> ws) then into lib.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");

        let lib_dir = ws.join("a/lib");
        write_manifest(&lib_dir, "lib", "lib", "");

        let root_dir = ws.join("a/b/c/root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "lib = { path = \"../../../lib\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        assert_eq!(graph.order.len(), 1);
        assert_eq!(graph.order.first().unwrap().name, "lib");
    }

    #[test]
    fn four_parent_traversals_rejected() {
        // One more than MAX_PARENT_TRAVERSAL (3) must be rejected.
        let tmp = tempfile::tempdir().unwrap();
        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "evil = { path = \"../../../../somewhere/lib\" }\n",
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

    fn make_dep(name: &str, dep_names: &[&str]) -> ResolvedDep {
        ResolvedDep {
            name: name.to_owned(),
            project_root: PathBuf::from(format!("/fake/{name}")),
            manifest: Manifest::from_str(
                &format!(
                    "[package]\nname = \"{name}\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n"
                ),
                "<test>",
            )
            .unwrap(),
            dep_names: dep_names.iter().map(|s| s.to_string()).collect(),
            source_hash: "deadbeef".to_owned(),
        }
    }

    #[test]
    fn parallel_levels_empty_graph() {
        let graph = ResolvedGraph { order: Vec::new() };
        let levels = parallel_levels(&graph);
        assert!(levels.is_empty());
    }

    #[test]
    fn parallel_levels_linear_chain() {
        // a -> b -> c (c is leaf, a depends on b, b depends on c)
        let graph = ResolvedGraph {
            order: vec![
                make_dep("c", &[]),
                make_dep("b", &["c"]),
                make_dep("a", &["b"]),
            ],
        };
        let levels = parallel_levels(&graph);
        assert_eq!(levels.len(), 3);
        assert_eq!(levels.first().unwrap().len(), 1);
        assert_eq!(levels.first().unwrap().first().unwrap().name, "c");
        assert_eq!(levels.get(1).unwrap().len(), 1);
        assert_eq!(levels.get(1).unwrap().first().unwrap().name, "b");
        assert_eq!(levels.get(2).unwrap().len(), 1);
        assert_eq!(levels.get(2).unwrap().first().unwrap().name, "a");
    }

    #[test]
    fn parallel_levels_diamond() {
        // shared <- [a, b] (both a and b depend on shared)
        let graph = ResolvedGraph {
            order: vec![
                make_dep("shared", &[]),
                make_dep("a", &["shared"]),
                make_dep("b", &["shared"]),
            ],
        };
        let levels = parallel_levels(&graph);
        assert_eq!(levels.len(), 2);
        // Level 0: shared (no deps)
        assert_eq!(levels.first().unwrap().len(), 1);
        assert_eq!(levels.first().unwrap().first().unwrap().name, "shared");
        // Level 1: a and b (both depend only on shared)
        assert_eq!(levels.get(1).unwrap().len(), 2);
        let level1_names: HashSet<&str> = levels
            .get(1)
            .unwrap()
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert!(level1_names.contains("a"));
        assert!(level1_names.contains("b"));
    }

    #[test]
    fn parallel_levels_wide() {
        // a, b, c all independent (no deps)
        let graph = ResolvedGraph {
            order: vec![make_dep("a", &[]), make_dep("b", &[]), make_dep("c", &[])],
        };
        let levels = parallel_levels(&graph);
        assert_eq!(levels.len(), 1);
        assert_eq!(levels.first().unwrap().len(), 3);
    }

    #[test]
    fn parallel_levels_preserves_all_deps() {
        // Every dep in the graph must appear exactly once across all levels.
        let graph = ResolvedGraph {
            order: vec![
                make_dep("x", &[]),
                make_dep("y", &["x"]),
                make_dep("z", &["x"]),
                make_dep("w", &["y", "z"]),
            ],
        };
        let levels = parallel_levels(&graph);
        let all_names: Vec<&str> = levels
            .iter()
            .flat_map(|level| level.iter().map(|d| d.name.as_str()))
            .collect();
        assert_eq!(all_names.len(), 4);
        let unique: HashSet<&str> = all_names.into_iter().collect();
        assert_eq!(unique.len(), 4);
        assert!(unique.contains("x"));
        assert!(unique.contains("y"));
        assert!(unique.contains("z"));
        assert!(unique.contains("w"));
    }

    // -- Smoke tests: parallel_levels through real resolve_dependencies --

    #[test]
    fn parallel_levels_single_dep_via_resolve() {
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
        let levels = parallel_levels(&graph);

        assert_eq!(levels.len(), 1);
        assert_eq!(levels.first().unwrap().len(), 1);
        assert_eq!(levels.first().unwrap().first().unwrap().name, "my-lib");
    }

    #[test]
    fn parallel_levels_diamond_via_resolve() {
        // shared has no deps; a and b both depend on shared; root depends on a and b.
        let tmp = tempfile::tempdir().unwrap();

        let shared_dir = tmp.path().join("shared");
        write_manifest(&shared_dir, "shared", "lib", "");

        let a_dir = tmp.path().join("a");
        write_manifest(&a_dir, "a", "lib", "shared = { path = \"../shared\" }\n");

        let b_dir = tmp.path().join("b");
        write_manifest(&b_dir, "b", "lib", "shared = { path = \"../shared\" }\n");

        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "a = { path = \"../a\" }\nb = { path = \"../b\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        let levels = parallel_levels(&graph);

        // Level 0: shared (leaf); Level 1: a and b (siblings).
        assert_eq!(levels.len(), 2);
        assert_eq!(levels.first().unwrap().len(), 1);
        assert_eq!(levels.first().unwrap().first().unwrap().name, "shared");
        assert_eq!(levels.get(1).unwrap().len(), 2);
        let level1_names: HashSet<&str> = levels
            .get(1)
            .unwrap()
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert!(level1_names.contains("a"));
        assert!(level1_names.contains("b"));
    }

    #[test]
    fn parallel_levels_wide_independent_via_resolve() {
        // Three independent libs with no deps between them.
        let tmp = tempfile::tempdir().unwrap();

        let x_dir = tmp.path().join("x");
        write_manifest(&x_dir, "x", "lib", "");

        let y_dir = tmp.path().join("y");
        write_manifest(&y_dir, "y", "lib", "");

        let z_dir = tmp.path().join("z");
        write_manifest(&z_dir, "z", "lib", "");

        let root_dir = tmp.path().join("root");
        write_manifest(
            &root_dir,
            "root",
            "bin",
            "x = { path = \"../x\" }\ny = { path = \"../y\" }\nz = { path = \"../z\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        let levels = parallel_levels(&graph);

        // All three are independent → single level.
        assert_eq!(levels.len(), 1);
        assert_eq!(levels.first().unwrap().len(), 3);
    }

    #[test]
    fn parallel_levels_chain_via_resolve() {
        // leaf → mid → root: each dep depends on the previous.
        let tmp = tempfile::tempdir().unwrap();

        let leaf_dir = tmp.path().join("leaf");
        write_manifest(&leaf_dir, "leaf", "lib", "");

        let mid_dir = tmp.path().join("mid");
        write_manifest(&mid_dir, "mid", "lib", "leaf = { path = \"../leaf\" }\n");

        let root_dir = tmp.path().join("root");
        write_manifest(&root_dir, "root", "bin", "mid = { path = \"../mid\" }\n");

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        let levels = parallel_levels(&graph);

        // leaf first, then mid — strictly sequential.
        assert_eq!(levels.len(), 2);
        assert_eq!(levels.first().unwrap().first().unwrap().name, "leaf");
        assert_eq!(levels.get(1).unwrap().first().unwrap().name, "mid");
    }

    #[test]
    fn parallel_levels_complex_graph_via_resolve() {
        // Models the issue description graph:
        //   app → [utils, models, logging]
        //   models → [shared]
        //   utils → [shared]
        //   logging → (no deps)
        //
        // Expected levels:
        //   Level 0: shared, logging  (independent leaves)
        //   Level 1: utils, models    (both only depend on shared)
        let tmp = tempfile::tempdir().unwrap();

        let shared_dir = tmp.path().join("shared");
        write_manifest(&shared_dir, "shared", "lib", "");

        let logging_dir = tmp.path().join("logging");
        write_manifest(&logging_dir, "logging", "lib", "");

        let utils_dir = tmp.path().join("utils");
        write_manifest(
            &utils_dir,
            "utils",
            "lib",
            "shared = { path = \"../shared\" }\n",
        );

        let models_dir = tmp.path().join("models");
        write_manifest(
            &models_dir,
            "models",
            "lib",
            "shared = { path = \"../shared\" }\n",
        );

        let root_dir = tmp.path().join("app");
        write_manifest(
            &root_dir,
            "app",
            "bin",
            "utils = { path = \"../utils\" }\nmodels = { path = \"../models\" }\nlogging = { path = \"../logging\" }\n",
        );

        let manifest = Manifest::from_path(&root_dir.join("konvoy.toml")).unwrap();
        let graph = resolve_dependencies(&root_dir, &manifest).unwrap();
        let levels = parallel_levels(&graph);

        // Level 0: shared and logging (both are leaves).
        // Level 1: utils and models (both depend only on shared).
        assert_eq!(levels.len(), 2, "expected 2 levels, got {levels:?}");

        let level0_names: HashSet<&str> = levels
            .first()
            .unwrap()
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert!(level0_names.contains("shared"), "level 0 missing shared");
        assert!(level0_names.contains("logging"), "level 0 missing logging");

        let level1_names: HashSet<&str> = levels
            .get(1)
            .unwrap()
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        assert!(level1_names.contains("utils"), "level 1 missing utils");
        assert!(level1_names.contains("models"), "level 1 missing models");

        // All 4 deps accounted for.
        let total: usize = levels.iter().map(|l| l.len()).sum();
        assert_eq!(total, 4);
    }

    mod property_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            /// Arbitrary strings as dep paths must never cause a panic.
            #[test]
            #[allow(clippy::unwrap_used)]
            fn arbitrary_path_never_panics(path in "\\PC*") {
                let dir = tempfile::tempdir().unwrap();
                let _ = resolve_dep_path(dir.path(), "test-dep", Some(&path));
            }

            /// Any path starting with `/` must be rejected.
            #[test]
            #[allow(clippy::unwrap_used)]
            fn absolute_paths_always_rejected(suffix in "[a-zA-Z0-9_./-]{0,50}") {
                let path = format!("/{suffix}");
                let dir = tempfile::tempdir().unwrap();
                let result = resolve_dep_path(dir.path(), "test-dep", Some(&path));
                prop_assert!(result.is_err());
                let err = result.unwrap_err().to_string();
                prop_assert!(
                    err.contains("escapes the project tree"),
                    "expected 'escapes the project tree' in error: {err}"
                );
            }

            /// Paths with more than MAX_PARENT_TRAVERSAL (3) leading `..` must be rejected.
            #[test]
            #[allow(clippy::unwrap_used)]
            fn deep_traversal_always_rejected(extra in 1..20usize, tail in "[a-z]{1,10}") {
                let prefix = "../".repeat(MAX_PARENT_TRAVERSAL + extra);
                let path = format!("{prefix}{tail}");
                let dir = tempfile::tempdir().unwrap();
                let result = resolve_dep_path(dir.path(), "test-dep", Some(&path));
                prop_assert!(result.is_err());
                let err = result.unwrap_err().to_string();
                prop_assert!(
                    err.contains("escapes the project tree"),
                    "expected 'escapes the project tree' in error: {err}"
                );
            }

            /// None path always produces an error (missing path).
            #[test]
            #[allow(clippy::unwrap_used)]
            fn none_path_always_errors(name in "[a-z][a-z0-9-]{0,20}") {
                let dir = tempfile::tempdir().unwrap();
                let result = resolve_dep_path(dir.path(), &name, None);
                prop_assert!(result.is_err());
            }
        }
    }
}
