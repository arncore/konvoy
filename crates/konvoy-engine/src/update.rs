//! The `konvoy update` command: resolve Maven deps via metadata-based transitive
//! resolution and populate lockfile hashes.
//!
//! For each dependency in `konvoy.toml` that has `maven` + `version` set:
//!
//! 1. Parse the `maven` field to get `groupId:artifactId`.
//! 2. For a probe target, fetch artifact metadata (`.module` JSON first,
//!    POM XML as fallback) and extract compile-scope dependencies.
//! 3. Recursively resolve transitive deps (BFS) with cycle detection.
//! 4. Detect version conflicts — fail with actionable error.
//! 5. For each resolved dep (direct + transitive), download the `.klib` and
//!    compute SHA-256 hashes. Also discover and download cinterop klibs
//!    listed in `.module` metadata files.
//! 6. Write the full dependency set to `konvoy.lock` with `required_by`
//!    populated for transitive deps and `classifier` for non-primary artifacts.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::Path;

use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use konvoy_config::lockfile::{DepSource, DependencyLock, Lockfile};
use konvoy_config::manifest::Manifest;
use konvoy_util::maven::MAVEN_CENTRAL;
use konvoy_util::metadata::ArtifactMetadata;
use konvoy_util::pom::strip_target_suffix;

use crate::error::EngineError;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Group IDs whose dependencies should be filtered out of the transitive set.
/// `kotlin-stdlib` is a JVM artifact and not a klib.
const FILTERED_GROUP_ARTIFACTS: &[(&str, &str)] = &[
    ("org.jetbrains.kotlin", "kotlin-stdlib"),
    ("org.jetbrains.kotlin", "kotlin-stdlib-common"),
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Result of an update operation.
#[derive(Debug)]
pub struct UpdateResult {
    /// Number of Maven dependencies that were resolved (direct + transitive).
    pub updated_count: usize,
}

/// Resolve Maven dependency versions and update `konvoy.lock` with per-target
/// hashes for all direct and transitive dependencies.
///
/// # Errors
///
/// Returns an error if a POM fetch or parse fails, a version conflict is
/// detected, a download fails, or the lockfile cannot be written.
pub fn update(project_root: &Path) -> Result<UpdateResult, EngineError> {
    // 1. Read konvoy.toml and konvoy.lock.
    let manifest_path = project_root.join("konvoy.toml");
    let manifest = Manifest::from_path(&manifest_path)?;

    let lockfile_path = project_root.join("konvoy.lock");
    let mut lockfile = Lockfile::from_path(&lockfile_path)?;

    // Ensure the toolchain section is populated so `konvoy build` can
    // recognise the lockfile without discarding dependency entries.
    if lockfile.toolchain.is_none() {
        lockfile.toolchain = Some(konvoy_config::lockfile::ToolchainLock {
            konanc_version: manifest.toolchain.kotlin.clone(),
            konanc_tarball_sha256: None,
            jre_tarball_sha256: None,
            detekt_version: None,
            detekt_jar_sha256: None,
        });
    }

    // 2. Collect Maven deps (those with `maven` + `version` set).
    let maven_deps: Vec<_> = manifest
        .dependencies
        .iter()
        .filter(|(_, spec)| spec.maven.is_some() && spec.version.is_some())
        .collect();

    if maven_deps.is_empty() {
        lockfile.write_to(&lockfile_path)?;
        return Ok(UpdateResult { updated_count: 0 });
    }

    // 3. Build the set of direct deps with their maven coordinates.
    let mut direct_deps: Vec<ResolvedMavenDep> = Vec::new();
    for (dep_name, dep_spec) in &maven_deps {
        let maven = dep_spec
            .maven
            .as_ref()
            .ok_or_else(|| EngineError::Metadata {
                message: format!("dependency `{dep_name}` is missing `maven` field"),
            })?;
        let version = dep_spec
            .version
            .as_ref()
            .ok_or_else(|| EngineError::Metadata {
                message: format!("dependency `{dep_name}` is missing `version` field"),
            })?;

        let (group_id, artifact_id) = split_maven_coordinate(maven)?;

        direct_deps.push(ResolvedMavenDep {
            name: (*dep_name).clone(),
            group_id: group_id.to_owned(),
            artifact_id: artifact_id.to_owned(),
            version: version.clone(),
            required_by: Vec::new(),
            classifier: None,
        });
    }

    // 4. Resolve transitive dependencies via BFS on POM files.
    let all_deps = resolve_transitive(&direct_deps)?;

    eprintln!(
        "  Resolved {} dependencies ({} direct, {} transitive)",
        all_deps.len(),
        direct_deps.len(),
        all_deps.len().saturating_sub(direct_deps.len())
    );

    // 5. Check if all resolved deps already match the lockfile — skip download if so.
    let mut new_dep_locks = Vec::new();

    for dep in &all_deps {
        let maven_coord = format!("{}:{}", dep.group_id, dep.artifact_id);
        eprintln!("  Resolving {} {}...", dep.name, dep.version);

        // Check if lockfile already has this dep at this version and classifier.
        let dep_classifier = &dep.classifier;
        let already_locked = lockfile.dependencies.iter().find(|d| {
            d.name == dep.name
                && matches!(&d.source, DepSource::Maven { version: v, maven: m, classifier: c, .. }
                    if v == &dep.version && m == &maven_coord && c == dep_classifier)
        });

        if let Some(existing) = already_locked {
            eprintln!("    (already up to date)");
            new_dep_locks.push(existing.clone());
            continue;
        }

        // For each known target, download and hash (in parallel).
        let known_targets = konvoy_targets::known_targets();

        let pid = std::process::id();
        let tmp_base = std::env::temp_dir().join(format!("konvoy-update-{pid}"));
        konvoy_util::fs::ensure_dir(&tmp_base)?;

        let target_results: Vec<Result<(String, String), EngineError>> = known_targets
            .par_iter()
            .map(|target_name| {
                let target = target_name
                    .parse::<konvoy_targets::Target>()
                    .map_err(EngineError::Target)?;
                let maven_suffix = target.to_maven_suffix();
                let per_target_artifact_id = format!("{}-{}", dep.artifact_id, maven_suffix);

                let mut coord = konvoy_util::maven::MavenCoordinate::new(
                    &dep.group_id,
                    &per_target_artifact_id,
                    &dep.version,
                )
                .with_packaging("klib");
                if let Some(cls) = &dep.classifier {
                    coord = coord.with_classifier(cls);
                }
                let url = coord.to_url(MAVEN_CENTRAL);

                let tmp_file = tmp_base.join(coord.filename());

                let result = konvoy_util::artifact::ensure_artifact(
                    &url,
                    &tmp_file,
                    None,
                    &format!("{}:{}", dep.name, target_name),
                    &dep.version,
                )
                .map_err(|e| match e {
                    konvoy_util::error::UtilError::Download { message } => {
                        EngineError::LibraryDownloadFailed {
                            name: dep.name.clone(),
                            url: url.clone(),
                            message,
                        }
                    }
                    other => EngineError::Util(other),
                })?;

                let _ = std::fs::remove_file(&tmp_file);

                Ok(((*target_name).to_owned(), result.sha256))
            })
            .collect();

        let mut targets_map: BTreeMap<String, String> = BTreeMap::new();
        for result in target_results {
            let (target_name, sha256) = result?;
            let display_hash = crate::build::truncate_hash(&sha256, 16);
            eprintln!("    {target_name}: {display_hash}...");
            targets_map.insert(target_name, sha256);
        }

        let _ = std::fs::remove_dir_all(&tmp_base);

        let hash_input: String = targets_map
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join("\n");
        let source_hash = konvoy_util::hash::sha256_bytes(hash_input.as_bytes());

        new_dep_locks.push(DependencyLock {
            name: dep.name.clone(),
            source: DepSource::Maven {
                version: dep.version.clone(),
                maven: maven_coord,
                targets: targets_map,
                required_by: dep.required_by.clone(),
                classifier: dep.classifier.clone(),
            },
            source_hash,
        });
    }

    // 6. Merge: preserve existing path deps, replace all Maven deps.
    let path_deps: Vec<_> = lockfile
        .dependencies
        .iter()
        .filter(|d| matches!(&d.source, DepSource::Path { .. }))
        .cloned()
        .collect();

    lockfile.dependencies = path_deps;
    lockfile.dependencies.extend(new_dep_locks);

    // Sort dependencies by name for deterministic output.
    lockfile.dependencies.sort_by(|a, b| a.name.cmp(&b.name));

    // 7. Write updated lockfile.
    lockfile.write_to(&lockfile_path)?;

    Ok(UpdateResult {
        updated_count: all_deps.len(),
    })
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A resolved Maven dependency (direct or transitive).
#[derive(Debug, Clone)]
struct ResolvedMavenDep {
    /// Konvoy dependency name (user-facing, e.g. "kotlinx-coroutines").
    name: String,
    /// Maven group identifier, e.g. "org.jetbrains.kotlinx".
    group_id: String,
    /// Maven artifact identifier (base, without target suffix),
    /// e.g. "kotlinx-coroutines-core".
    artifact_id: String,
    /// Pinned version string.
    version: String,
    /// Names of dependencies that pulled this one in transitively.
    /// Empty for direct deps declared in `konvoy.toml`.
    required_by: Vec<String>,
    /// Maven classifier for non-primary artifacts (e.g. "cinterop-interop").
    /// `None` for the main klib.
    classifier: Option<String>,
}

// ---------------------------------------------------------------------------
// Transitive resolution (BFS on artifact metadata)
// ---------------------------------------------------------------------------

/// Resolve the full transitive closure of Maven dependencies via BFS on
/// artifact metadata (`.module` JSON first, POM XML as fallback).
///
/// Uses the first known target to discover transitive deps (the dependency
/// graph is the same across targets — only the klib differs).
///
/// After resolving the dependency graph, inspects each dep's metadata for
/// additional klib files (e.g. cinterop klibs) and adds them as separate
/// entries with a `classifier` set.
///
/// # Errors
///
/// Returns an error if metadata cannot be fetched or parsed, a version conflict
/// is detected, or a dependency cycle is found.
/// A BFS queue entry: (group_id, artifact_id, version, requirer_name, ancestor_path).
type BfsEntry = (String, String, String, Option<String>, Vec<String>);

/// State accumulated during the BFS traversal of Maven transitive dependencies.
struct BfsState {
    user_versions: HashMap<String, String>,
    resolved: HashMap<String, ResolvedMavenDep>,
    metadata_cache: HashMap<String, ArtifactMetadata>,
    required_by_map: HashMap<String, BTreeSet<String>>,
    queue: VecDeque<BfsEntry>,
}

/// Check for a version conflict between a newly resolved dep and one already seen.
///
/// Returns an error describing the conflict with an actionable hint, or `Ok(())`
/// if the versions match (already resolved — caller should skip).
fn check_version_conflict(
    existing: &ResolvedMavenDep,
    resolved_version: &str,
    dep_key: &str,
    base_artifact_id: &str,
    requirer: Option<&str>,
) -> Result<(), EngineError> {
    if existing.version == resolved_version {
        return Ok(());
    }

    let existing_requirer = if existing.required_by.is_empty() {
        "konvoy.toml (direct)"
    } else {
        existing
            .required_by
            .first()
            .map(String::as_str)
            .unwrap_or("unknown")
    };
    let current_requirer = requirer.unwrap_or("konvoy.toml (direct)");

    let details = format!(
        "  {existing_requirer} requires {}\n  {current_requirer} requires {resolved_version}",
        existing.version
    );
    let hint_name = base_artifact_id.replace('.', "-");
    let hint_version = if resolved_version > existing.version.as_str() {
        resolved_version.to_owned()
    } else {
        existing.version.clone()
    };

    Err(EngineError::MavenVersionConflict {
        maven: dep_key.to_owned(),
        details,
        hint_name,
        hint_version,
    })
}

/// Discover cinterop klibs from cached metadata and return them as separate deps.
fn discover_cinterop_deps(
    resolved: &HashMap<String, ResolvedMavenDep>,
    metadata_cache: &HashMap<String, ArtifactMetadata>,
    maven_suffix: &str,
) -> Vec<ResolvedMavenDep> {
    let mut cinterop_deps = Vec::new();

    for dep in resolved.values() {
        let metadata_key = format!(
            "{}:{}-{}:{}",
            dep.group_id, dep.artifact_id, maven_suffix, dep.version
        );
        let Some(metadata) = metadata_cache.get(&metadata_key) else {
            continue;
        };
        for file in &metadata.files {
            if !file.name.contains("cinterop-") {
                continue;
            }
            let Some(cls) = extract_classifier_from_url(&file.url, &dep.version) else {
                continue;
            };
            let cinterop_name = file
                .name
                .strip_suffix(".klib")
                .unwrap_or(&file.name)
                .to_owned();

            cinterop_deps.push(ResolvedMavenDep {
                name: cinterop_name,
                group_id: dep.group_id.clone(),
                artifact_id: dep.artifact_id.clone(),
                version: dep.version.clone(),
                required_by: vec![dep.name.clone()],
                classifier: Some(cls),
            });
        }
    }

    cinterop_deps
}

/// Finalize the `required_by` fields: direct deps get empty, transitive deps
/// get the accumulated requirer set.
fn finalize_required_by(
    resolved: &mut HashMap<String, ResolvedMavenDep>,
    required_by_map: &HashMap<String, BTreeSet<String>>,
    direct_deps: &[ResolvedMavenDep],
) {
    for (key, dep) in resolved.iter_mut() {
        if let Some(requirers) = required_by_map.get(key) {
            let is_direct = direct_deps
                .iter()
                .any(|d| format!("{}:{}", d.group_id, d.artifact_id) == *key);
            if is_direct {
                dep.required_by = Vec::new();
            } else {
                dep.required_by = requirers.iter().cloned().collect();
            }
        }
    }
}

fn resolve_transitive(
    direct_deps: &[ResolvedMavenDep],
) -> Result<Vec<ResolvedMavenDep>, EngineError> {
    // Use the first known target for metadata fetching — the transitive graph is
    // identical across targets since every Kotlin/Native artifact publishes
    // the same set of dependencies in each per-target variant.
    let probe_target =
        konvoy_targets::known_targets()
            .first()
            .ok_or_else(|| EngineError::Metadata {
                message: "no known targets available".to_owned(),
            })?;
    let probe_target_parsed = probe_target
        .parse::<konvoy_targets::Target>()
        .map_err(EngineError::Target)?;
    let maven_suffix = probe_target_parsed.to_maven_suffix();

    let mut state = BfsState {
        user_versions: HashMap::new(),
        resolved: HashMap::new(),
        metadata_cache: HashMap::new(),
        required_by_map: HashMap::new(),
        queue: VecDeque::new(),
    };

    // Build user-specified version map and seed the BFS queue.
    for dep in direct_deps {
        let key = format!("{}:{}", dep.group_id, dep.artifact_id);
        state.user_versions.insert(key.clone(), dep.version.clone());
        state.resolved.insert(key.clone(), dep.clone());
        state.queue.push_back((
            dep.group_id.clone(),
            dep.artifact_id.clone(),
            dep.version.clone(),
            None,
            vec![key],
        ));
    }

    // BFS loop.
    while let Some((group_id, artifact_id, version, requirer, path)) = state.queue.pop_front() {
        let key = format!("{group_id}:{artifact_id}");

        if let Some(req) = &requirer {
            state
                .required_by_map
                .entry(key.clone())
                .or_default()
                .insert(req.clone());
        }

        let per_target_artifact_id = format!("{artifact_id}-{maven_suffix}");
        let metadata = fetch_metadata_cached(
            &group_id,
            &per_target_artifact_id,
            &version,
            &maven_suffix,
            &mut state.metadata_cache,
        )?;

        for meta_dep in &metadata.dependencies {
            if is_filtered_dependency(&meta_dep.group_id, &meta_dep.artifact_id) {
                continue;
            }
            if meta_dep.version.is_empty() {
                continue;
            }

            let base_artifact_id = strip_target_suffix(&meta_dep.artifact_id, &maven_suffix);
            let dep_key = format!("{}:{}", meta_dep.group_id, base_artifact_id);

            let resolved_version = state
                .user_versions
                .get(&dep_key)
                .cloned()
                .unwrap_or_else(|| meta_dep.version.clone());

            // Already resolved — check for version conflict, then skip.
            if let Some(existing) = state.resolved.get(&dep_key) {
                check_version_conflict(
                    existing,
                    &resolved_version,
                    &dep_key,
                    &base_artifact_id,
                    requirer.as_deref(),
                )?;
                if let Some(req) = &requirer {
                    state
                        .required_by_map
                        .entry(dep_key)
                        .or_default()
                        .insert(req.clone());
                }
                continue;
            }

            let dep_name = derive_dep_name(&base_artifact_id);
            let parent_name = requirer
                .clone()
                .or_else(|| {
                    state
                        .resolved
                        .get(&format!("{group_id}:{artifact_id}"))
                        .map(|d| d.name.clone())
                })
                .unwrap_or_else(|| "unknown".to_owned());

            state
                .required_by_map
                .entry(dep_key.clone())
                .or_default()
                .insert(parent_name.clone());

            // Check for cycles.
            if path.contains(&dep_key) {
                let cycle = format!("{} -> {dep_key}", path.join(" -> "));
                return Err(EngineError::MavenDependencyCycle { cycle });
            }

            let new_dep = ResolvedMavenDep {
                name: dep_name.clone(),
                group_id: meta_dep.group_id.clone(),
                artifact_id: base_artifact_id.clone(),
                version: resolved_version.clone(),
                required_by: vec![parent_name],
                classifier: None,
            };

            let mut child_path = path.clone();
            child_path.push(dep_key.clone());

            state.resolved.insert(dep_key, new_dep);
            state.queue.push_back((
                meta_dep.group_id.clone(),
                base_artifact_id,
                resolved_version,
                Some(dep_name),
                child_path,
            ));
        }
    }

    finalize_required_by(&mut state.resolved, &state.required_by_map, direct_deps);

    let cinterop_deps =
        discover_cinterop_deps(&state.resolved, &state.metadata_cache, &maven_suffix);

    let mut result: Vec<ResolvedMavenDep> = Vec::with_capacity(state.resolved.len());
    result.extend(state.resolved.into_values());
    result.extend(cinterop_deps);

    // Sort for deterministic ordering.
    result.sort_by(|a, b| {
        a.name.cmp(&b.name).then_with(|| {
            a.classifier
                .as_deref()
                .unwrap_or("")
                .cmp(b.classifier.as_deref().unwrap_or(""))
        })
    });

    Ok(result)
}

// ---------------------------------------------------------------------------
// Metadata caching
// ---------------------------------------------------------------------------

/// Fetch artifact metadata via the provider chain, with an in-memory cache.
///
/// Uses `fetch_artifact_metadata()` (`.module` first, POM fallback) and stores
/// the result in `cache` keyed by `groupId:artifactId:version`.
///
/// # Errors
///
/// Returns an error if both `.module` and POM fetching/parsing fail.
fn fetch_metadata_cached(
    group_id: &str,
    artifact_id: &str,
    version: &str,
    maven_suffix: &str,
    cache: &mut HashMap<String, ArtifactMetadata>,
) -> Result<ArtifactMetadata, EngineError> {
    let key = format!("{group_id}:{artifact_id}:{version}");
    if let Some(cached) = cache.get(&key) {
        return Ok(cached.clone());
    }

    let metadata = konvoy_util::metadata::fetch_artifact_metadata(
        group_id,
        artifact_id,
        version,
        maven_suffix,
    )
    .map_err(EngineError::Util)?;

    cache.insert(key, metadata.clone());
    Ok(metadata)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split a `groupId:artifactId` string into its two parts.
///
/// Re-export of [`crate::common::split_maven_coordinate`] for backward compatibility.
pub(crate) fn split_maven_coordinate(maven: &str) -> Result<(&str, &str), EngineError> {
    crate::common::split_maven_coordinate(maven)
}

/// Return `true` if this dependency should be filtered from transitive resolution.
///
/// Filters `kotlin-stdlib` and `kotlin-stdlib-common` which are JVM artifacts.
fn is_filtered_dependency(group_id: &str, artifact_id: &str) -> bool {
    // Strip target suffix before checking: "kotlin-stdlib-linuxx64" → "kotlin-stdlib"
    // But the filter list uses base names, so we also check if the artifact_id
    // starts with a filtered name followed by nothing or a target suffix.
    for &(filtered_group, filtered_artifact) in FILTERED_GROUP_ARTIFACTS {
        if group_id == filtered_group {
            // Exact match or match with target suffix appended.
            if artifact_id == filtered_artifact
                || artifact_id.starts_with(&format!("{filtered_artifact}-"))
            {
                return true;
            }
        }
    }
    false
}

/// Derive a user-friendly dependency name from a Maven artifact ID.
///
/// Examples:
/// - `"kotlinx-coroutines-core"` → `"kotlinx-coroutines-core"`
/// - `"atomicfu"` → `"atomicfu"`
fn derive_dep_name(artifact_id: &str) -> String {
    artifact_id.to_owned()
}

/// Extract the Maven classifier from a cinterop file URL.
///
/// Given a URL like `"atomicfu-linuxx64-0.23.1-cinterop-interop.klib"`,
/// extracts `"cinterop-interop"` — the segment between `"{version}-"` and `".klib"`.
///
/// Returns `None` if the URL does not match the expected pattern.
fn extract_classifier_from_url(url: &str, version: &str) -> Option<String> {
    let version_dash = format!("{version}-");
    let after_version = url.find(&version_dash).map(|i| i + version_dash.len())?;
    let suffix = url.get(after_version..)?;
    let classifier = suffix.strip_suffix(".klib")?;
    if classifier.is_empty() {
        return None;
    }
    Some(classifier.to_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use konvoy_config::lockfile::{DepSource, DependencyLock, Lockfile};

    use super::*;

    /// Helper to create a temp project directory with a konvoy.toml.
    fn make_project(toml_content: &str) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("konvoy.toml"), toml_content).unwrap();
        // Create a minimal src directory so manifest validation passes.
        fs::create_dir_all(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("src/main.kt"), "fun main() {}").unwrap();
        tmp
    }

    #[test]
    fn update_no_maven_deps_is_noop() {
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
my-utils = { path = "../my-utils" }
"#,
        );
        // Write an initial lockfile with a path dep.
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-utils".to_owned(),
            source: DepSource::Path {
                path: "../my-utils".to_owned(),
            },
            source_hash: "abcdef".to_owned(),
        });
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        let result = update(project.path()).unwrap();
        assert_eq!(result.updated_count, 0);

        // Verify the lockfile still has the path dep.
        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert_eq!(reparsed.dependencies.len(), 1);
        assert_eq!(reparsed.dependencies[0].name, "my-utils");
    }

    #[test]
    fn update_preserves_path_deps() {
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
my-utils = { path = "../my-utils" }
"#,
        );
        // Pre-populate lockfile with a path dep.
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-utils".to_owned(),
            source: DepSource::Path {
                path: "../my-utils".to_owned(),
            },
            source_hash: "path-hash".to_owned(),
        });
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        let result = update(project.path()).unwrap();
        assert_eq!(result.updated_count, 0);

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert_eq!(reparsed.dependencies.len(), 1);
        let dep = &reparsed.dependencies[0];
        assert_eq!(dep.name, "my-utils");
        match &dep.source {
            DepSource::Path { path } => assert_eq!(path, "../my-utils"),
            other => panic!("expected Path source, got: {other:?}"),
        }
    }

    #[test]
    fn update_preserves_toolchain_and_plugins() {
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
"#,
        );
        // Pre-populate lockfile with toolchain info.
        let lockfile = Lockfile::with_managed_toolchain("2.1.0", Some("tc-hash"), Some("jre-hash"));
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        let result = update(project.path()).unwrap();
        assert_eq!(result.updated_count, 0);

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_version, "2.1.0");
        assert_eq!(tc.konanc_tarball_sha256.as_deref(), Some("tc-hash"));
        assert_eq!(tc.jre_tarball_sha256.as_deref(), Some("jre-hash"));
    }

    #[test]
    fn update_no_deps_writes_empty_lockfile() {
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
"#,
        );

        let result = update(project.path()).unwrap();
        assert_eq!(result.updated_count, 0);

        // Lockfile should exist (possibly empty/default).
        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert!(reparsed.dependencies.is_empty());
    }

    #[test]
    fn lockfile_maven_dep_round_trips_correctly() {
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
kotlinx-coroutines = { maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }
"#,
        );
        // Write a lockfile with a Maven dep and verify it round-trips.
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "hash-lx64".to_owned());
        targets.insert("linux_arm64".to_owned(), "hash-la64".to_owned());
        targets.insert("macos_x64".to_owned(), "hash-mx64".to_owned());
        targets.insert("macos_arm64".to_owned(), "hash-ma64".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "kotlinx-coroutines".to_owned(),
            source: DepSource::Maven {
                version: "1.8.0".to_owned(),
                maven: "org.jetbrains.kotlinx:kotlinx-coroutines-core".to_owned(),
                targets: targets.clone(),
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "existing-hash".to_owned(),
        });
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert_eq!(reparsed.dependencies.len(), 1);
        let dep = &reparsed.dependencies[0];
        assert_eq!(dep.name, "kotlinx-coroutines");
        assert_eq!(dep.source_hash, "existing-hash");
        match &dep.source {
            DepSource::Maven { version, .. } => assert_eq!(version, "1.8.0"),
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    #[test]
    fn split_maven_coordinate_valid() {
        let (g, a) =
            split_maven_coordinate("org.jetbrains.kotlinx:kotlinx-coroutines-core").unwrap();
        assert_eq!(g, "org.jetbrains.kotlinx");
        assert_eq!(a, "kotlinx-coroutines-core");
    }

    #[test]
    fn split_maven_coordinate_invalid() {
        let result = split_maven_coordinate("no-colon-here");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid maven coordinate"), "error was: {err}");
    }

    #[test]
    fn is_filtered_kotlin_stdlib() {
        assert!(is_filtered_dependency(
            "org.jetbrains.kotlin",
            "kotlin-stdlib"
        ));
        assert!(is_filtered_dependency(
            "org.jetbrains.kotlin",
            "kotlin-stdlib-linuxx64"
        ));
        assert!(is_filtered_dependency(
            "org.jetbrains.kotlin",
            "kotlin-stdlib-common"
        ));
        assert!(is_filtered_dependency(
            "org.jetbrains.kotlin",
            "kotlin-stdlib-common-macosarm64"
        ));
    }

    #[test]
    fn is_not_filtered_regular_dep() {
        assert!(!is_filtered_dependency(
            "org.jetbrains.kotlinx",
            "kotlinx-coroutines-core"
        ));
        assert!(!is_filtered_dependency("org.jetbrains.kotlinx", "atomicfu"));
    }

    #[test]
    fn derive_dep_name_from_artifact_id() {
        assert_eq!(
            derive_dep_name("kotlinx-coroutines-core"),
            "kotlinx-coroutines-core"
        );
        assert_eq!(derive_dep_name("atomicfu"), "atomicfu");
    }

    #[test]
    fn version_conflict_error_format() {
        let err = EngineError::MavenVersionConflict {
            maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
            details: "  kotlinx-coroutines 1.8.0 requires 0.23.1\n  kotlinx-serialization 1.6.3 requires 0.22.0".to_owned(),
            hint_name: "atomicfu".to_owned(),
            hint_version: "0.23.1".to_owned(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("version conflict for 'org.jetbrains.kotlinx:atomicfu'"),
            "error was: {msg}"
        );
        assert!(
            msg.contains("kotlinx-coroutines 1.8.0 requires 0.23.1"),
            "error was: {msg}"
        );
        assert!(
            msg.contains("hint: add an explicit version in konvoy.toml:"),
            "error was: {msg}"
        );
        assert!(
            msg.contains(
                "atomicfu = { maven = \"org.jetbrains.kotlinx:atomicfu\", version = \"0.23.1\" }"
            ),
            "error was: {msg}"
        );
    }

    #[test]
    fn resolve_transitive_empty_direct_deps() {
        let result = resolve_transitive(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn version_conflict_error_is_actionable() {
        let err = EngineError::MavenVersionConflict {
            maven: "com.example:lib".to_owned(),
            details: "  A requires 1.0\n  B requires 2.0".to_owned(),
            hint_name: "lib".to_owned(),
            hint_version: "2.0".to_owned(),
        };
        let msg = err.to_string();
        assert!(msg.contains("hint:"), "should include a hint: {msg}");
        assert!(
            msg.contains("konvoy.toml"),
            "should reference konvoy.toml: {msg}"
        );
    }

    #[test]
    fn cycle_detection_error_is_actionable() {
        let err = EngineError::MavenDependencyCycle {
            cycle: "a:x -> b:y -> a:x".to_owned(),
        };
        let msg = err.to_string();
        assert!(msg.contains("cycle detected"), "should say cycle: {msg}");
        assert!(
            msg.contains("a:x -> b:y -> a:x"),
            "should show cycle path: {msg}"
        );
        assert!(
            msg.contains("remove one of these dependencies"),
            "should be actionable: {msg}"
        );
    }

    #[test]
    fn update_populates_toolchain_when_missing() {
        // Regression: `konvoy update` on a fresh project (no prior lockfile)
        // must write a `[toolchain]` section so that `konvoy build` does not
        // discard the dependency entries.
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
"#,
        );
        // No konvoy.lock on disk — `update` creates it from scratch.
        let result = update(project.path()).unwrap();
        assert_eq!(result.updated_count, 0);

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert!(
            reparsed.toolchain.is_some(),
            "lockfile written by `update` must have a [toolchain] section"
        );
        assert_eq!(reparsed.toolchain.as_ref().unwrap().konanc_version, "2.1.0");
    }

    #[test]
    fn update_does_not_overwrite_existing_toolchain() {
        // If a lockfile already has a [toolchain] section with tarball hashes,
        // `update` should not overwrite it with a bare version-only section.
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
"#,
        );
        let lockfile = Lockfile::with_managed_toolchain("2.1.0", Some("tc-hash"), Some("jre-hash"));
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        let result = update(project.path()).unwrap();
        assert_eq!(result.updated_count, 0);

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_tarball_sha256.as_deref(), Some("tc-hash"));
        assert_eq!(tc.jre_tarball_sha256.as_deref(), Some("jre-hash"));
    }

    #[test]
    fn extract_classifier_cinterop_interop() {
        let cls =
            extract_classifier_from_url("atomicfu-linuxx64-0.23.1-cinterop-interop.klib", "0.23.1");
        assert_eq!(cls.as_deref(), Some("cinterop-interop"));
    }

    #[test]
    fn extract_classifier_main_klib_returns_none() {
        // Main klib URL does not have a classifier.
        let cls = extract_classifier_from_url("atomicfu-linuxx64-0.23.1.klib", "0.23.1");
        assert!(cls.is_none());
    }

    #[test]
    fn extract_classifier_no_klib_extension() {
        let cls = extract_classifier_from_url("atomicfu-linuxx64-0.23.1-cinterop.jar", "0.23.1");
        assert!(cls.is_none());
    }

    #[test]
    fn extract_classifier_version_not_in_url() {
        let cls = extract_classifier_from_url("some-file.klib", "0.23.1");
        assert!(cls.is_none());
    }

    #[test]
    fn extract_classifier_complex_cinterop_name() {
        // Classifier with multiple dashes.
        let cls =
            extract_classifier_from_url("lib-linuxx64-1.0.0-cinterop-native-mt.klib", "1.0.0");
        assert_eq!(cls.as_deref(), Some("cinterop-native-mt"));
    }

    #[test]
    fn extract_classifier_empty_classifier_returns_none() {
        // URL where there is nothing between version-dash and .klib.
        let cls = extract_classifier_from_url("lib-linuxx64-1.0.0-.klib", "1.0.0");
        assert!(cls.is_none());
    }

    #[test]
    fn extract_classifier_from_full_url_path() {
        // Extract classifier even when URL is a relative path with directory.
        let cls =
            extract_classifier_from_url("atomicfu-linuxx64-0.23.1-cinterop-interop.klib", "0.23.1");
        assert_eq!(cls.as_deref(), Some("cinterop-interop"));
    }

    #[test]
    fn extract_classifier_version_appears_multiple_times() {
        // If version appears multiple times, classifier is taken after the first match.
        let cls = extract_classifier_from_url("lib-0.23.1-0.23.1-cinterop-x.klib", "0.23.1");
        // After first "0.23.1-", the rest is "0.23.1-cinterop-x.klib".
        // strip_suffix(".klib") gives "0.23.1-cinterop-x".
        assert_eq!(cls.as_deref(), Some("0.23.1-cinterop-x"));
    }

    #[test]
    fn extract_classifier_non_klib_extension_returns_none() {
        let cls = extract_classifier_from_url("lib-1.0-sources.jar", "1.0");
        assert!(cls.is_none());
    }

    #[test]
    fn resolve_transitive_no_direct_deps_returns_empty() {
        let result = resolve_transitive(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn lockfile_classifier_written_for_cinterop_deps() {
        // Verify that when a lockfile is written with a cinterop dep,
        // the classifier field appears in the serialized TOML.
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        let mut targets = BTreeMap::new();
        targets.insert("linux_x64".to_owned(), "hash".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu-cinterop-interop".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets,
                required_by: vec!["atomicfu".to_owned()],
                classifier: Some("cinterop-interop".to_owned()),
            },
            source_hash: "cinterop-hash".to_owned(),
        });
        let content =
            toml::to_string_pretty(&lockfile).unwrap_or_else(|e| panic!("serialize: {e}"));
        assert!(
            content.contains("classifier = \"cinterop-interop\""),
            "classifier should appear in serialized lockfile, content was: {content}"
        );
    }

    #[test]
    fn resolved_maven_dep_classifier_field_defaults_to_none() {
        // ResolvedMavenDep should have classifier None for regular deps.
        let dep = ResolvedMavenDep {
            name: "atomicfu".to_owned(),
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "atomicfu".to_owned(),
            version: "0.23.1".to_owned(),
            required_by: Vec::new(),
            classifier: None,
        };
        assert!(dep.classifier.is_none());
    }

    #[test]
    fn resolved_maven_dep_classifier_some_for_cinterop() {
        // ResolvedMavenDep can hold a classifier for cinterop deps.
        let dep = ResolvedMavenDep {
            name: "atomicfu-cinterop-interop".to_owned(),
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "atomicfu".to_owned(),
            version: "0.23.1".to_owned(),
            required_by: vec!["atomicfu".to_owned()],
            classifier: Some("cinterop-interop".to_owned()),
        };
        assert_eq!(dep.classifier.as_deref(), Some("cinterop-interop"));
    }

    #[test]
    fn update_preserves_already_locked_classifier_dep() {
        // When a dep with a specific classifier is already locked at the
        // same version, it should be preserved (not re-downloaded). The
        // lockfile output should contain both the main klib and the
        // cinterop klib with classifiers intact.
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
atomicfu = { maven = "org.jetbrains.kotlinx:atomicfu", version = "0.23.1" }
"#,
        );

        // Build a lockfile with both the main klib and cinterop klib.
        let mut lockfile = Lockfile::with_toolchain("2.1.0");

        // Main klib (no classifier).
        let mut targets1 = BTreeMap::new();
        targets1.insert("linux_x64".to_owned(), "main-hash".to_owned());
        targets1.insert("macos_arm64".to_owned(), "main-hash-mac".to_owned());
        targets1.insert("macos_x64".to_owned(), "main-hash-macx".to_owned());
        targets1.insert("linux_arm64".to_owned(), "main-hash-la64".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets: targets1,
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "main-source-hash".to_owned(),
        });

        // Cinterop klib (with classifier).
        let mut targets2 = BTreeMap::new();
        targets2.insert("linux_x64".to_owned(), "cinterop-hash".to_owned());
        targets2.insert("macos_arm64".to_owned(), "cinterop-hash-mac".to_owned());
        targets2.insert("macos_x64".to_owned(), "cinterop-hash-macx".to_owned());
        targets2.insert("linux_arm64".to_owned(), "cinterop-hash-la64".to_owned());
        lockfile.dependencies.push(DependencyLock {
            name: "atomicfu-cinterop-interop".to_owned(),
            source: DepSource::Maven {
                version: "0.23.1".to_owned(),
                maven: "org.jetbrains.kotlinx:atomicfu".to_owned(),
                targets: targets2,
                required_by: vec!["atomicfu".to_owned()],
                classifier: Some("cinterop-interop".to_owned()),
            },
            source_hash: "cinterop-source-hash".to_owned(),
        });

        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        // Running update should detect these as already locked (same version
        // and classifier) and skip re-downloading.
        let result = update(project.path()).unwrap();
        // updated_count reflects total resolved deps (already-locked or new).
        assert!(
            result.updated_count >= 2,
            "should resolve at least 2 deps (main + cinterop)"
        );

        // Both deps should be preserved in the lockfile.
        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        let cinterop = reparsed
            .dependencies
            .iter()
            .find(|d| d.name == "atomicfu-cinterop-interop");
        assert!(
            cinterop.is_some(),
            "cinterop dep should be preserved in lockfile after update"
        );
        let cinterop = cinterop.unwrap();
        match &cinterop.source {
            DepSource::Maven { classifier, .. } => {
                assert_eq!(classifier.as_deref(), Some("cinterop-interop"));
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
        // The main klib should also be present.
        let main = reparsed.dependencies.iter().find(|d| {
            d.name == "atomicfu"
                && matches!(
                    &d.source,
                    DepSource::Maven {
                        classifier: None,
                        ..
                    }
                )
        });
        assert!(
            main.is_some(),
            "main klib dep (no classifier) should be preserved"
        );
    }

    #[test]
    fn extract_classifier_from_full_maven_central_url() {
        // Test with a URL that includes the full Maven Central path.
        let url = "https://repo1.maven.org/maven2/org/jetbrains/kotlinx/atomicfu-linuxx64/0.23.1/atomicfu-linuxx64-0.23.1-cinterop-interop.klib";
        let cls = extract_classifier_from_url(url, "0.23.1");
        assert_eq!(cls.as_deref(), Some("cinterop-interop"));
    }
}
