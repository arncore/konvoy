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
use std::path::{Path, PathBuf};

use rayon::prelude::{IndexedParallelIterator, IntoParallelRefIterator, ParallelIterator};

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
// Per-dep download helpers
// ---------------------------------------------------------------------------

/// Build a placeholder lock entry used while the real one is being downloaded.
///
/// The placeholder is overwritten before the lockfile is written, so its
/// `targets`/`source_hash` are never observed.
fn placeholder_lock(dep: &ResolvedMavenDep, maven_coord: &str) -> DependencyLock {
    DependencyLock {
        name: dep.name.clone(),
        source: DepSource::Maven {
            version: dep.version.clone(),
            maven: maven_coord.to_owned(),
            targets: BTreeMap::new(),
            required_by: dep.required_by.clone(),
            classifier: dep.classifier.clone(),
        },
        source_hash: String::new(),
    }
}

/// Download a Maven dep's klib for every known target and produce its lock entry.
///
/// Klibs are written directly into the shared Maven cache at
/// `~/.konvoy/cache/maven/.../<artifact>-<version>.klib` so subsequent
/// `konvoy build` runs reuse the verified file. Per-target downloads run in
/// parallel via rayon and each renders into a pre-allocated progress bar (one
/// per target, in `KNOWN_TARGETS` order) so the on-screen layout stays in
/// the same stable rows regardless of completion order.
///
/// `bars` must have one entry per `KNOWN_TARGETS` element; caller-enforced
/// via the zip in `update()`.
fn download_dep(
    dep: &ResolvedMavenDep,
    cache_root: &Path,
    bars: &[konvoy_util::progress::DownloadBar],
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<DependencyLock, EngineError> {
    let known_targets = konvoy_targets::KNOWN_TARGETS;
    let maven_coord = dep.key();

    let target_results: Vec<Result<(konvoy_targets::Target, String), EngineError>> = known_targets
        .par_iter()
        .zip(bars.par_iter())
        .map(|(&target, bar)| download_target_klib(dep, target, cache_root, bar, resolver))
        .collect();

    let mut targets_map: BTreeMap<String, String> = BTreeMap::new();
    for result in target_results {
        let (target, sha256) = result?;
        targets_map.insert(target.to_string(), sha256);
    }

    let hash_input: String = targets_map
        .iter()
        .map(|(k, v)| format!("{k}:{v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let source_hash = konvoy_util::hash::sha256_bytes(hash_input.as_bytes());

    Ok(DependencyLock {
        name: dep.name.clone(),
        source: DepSource::Maven {
            version: dep.version.clone(),
            maven: maven_coord,
            targets: targets_map,
            required_by: dep.required_by.clone(),
            classifier: dep.classifier.clone(),
        },
        source_hash,
    })
}

/// Download a single dep's klib for one target and return `(target, sha256)`.
///
/// Writes the klib to the shared Maven cache so `konvoy build` can reuse it
/// without re-downloading. The bar is provided by the caller so the on-screen
/// row is fixed and stable.
fn download_target_klib(
    dep: &ResolvedMavenDep,
    target: konvoy_targets::Target,
    cache_root: &Path,
    progress: &konvoy_util::progress::DownloadBar,
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<(konvoy_targets::Target, String), EngineError> {
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
    let dest = coord.cache_path(cache_root);
    let label = format!("{}:{}", dep.name, target);

    let result = resolver
        .fetch_artifact(&url, &dest, None, &label, Some(progress))
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

    Ok((target, result.sha256))
}

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
pub fn update(
    project_root: &Path,
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<UpdateResult, EngineError> {
    let manifest = Manifest::from_path(&project_root.join("konvoy.toml"))?;
    // Resolve the path-dependency graph once; the Maven union spans it.
    let dep_graph = crate::resolve::resolve_dependencies(project_root, &manifest)?;
    update_with_graph(project_root, &manifest, &dep_graph, resolver)
}

/// [`update`] with the path-dependency graph already resolved.
///
/// The build's auto-update path has already resolved (and source-hashed) the
/// graph at `resolve_build_context`, so it calls this directly to avoid walking
/// and hashing every path-dep's source tree a second time on a cold build.
pub(crate) fn update_with_graph(
    project_root: &Path,
    manifest: &Manifest,
    dep_graph: &crate::resolve::ResolvedGraph,
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<UpdateResult, EngineError> {
    // 1. Read konvoy.lock.
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

    // 2. Collect the DIRECT Maven deps as the deduped union across the whole
    //    build graph — the root plus every path-dependency. A path-dep's
    //    `[dependencies]` must be resolved and pinned in the root lock too,
    //    because the dep's own compile links the klibs it declares (the
    //    `[dependencies]` analogue of the path-dep `[plugins]` fix, #293).
    //    Cross-project version clashes are surfaced (libraries are linked).
    let mut projects: Vec<(&str, &Manifest)> = vec![("konvoy.toml", manifest)];
    projects.extend(
        dep_graph
            .order
            .iter()
            .map(|dep| (dep.name.as_str(), &dep.manifest)),
    );
    let direct_deps = collect_graph_direct_maven_deps(projects)?;

    if direct_deps.is_empty() {
        // No Maven deps anywhere in the graph — prune any stale Maven pins (the
        // last Maven dep may have just been removed) while preserving path-dep
        // locks, then write. Without this, a removed Maven dep's pin would
        // linger and still be linked/enforced under `--locked`.
        lockfile
            .dependencies
            .retain(|d| matches!(&d.source, DepSource::Path { .. }));
        lockfile.write_to(&lockfile_path)?;
        return Ok(UpdateResult { updated_count: 0 });
    }

    // 4. Resolve transitive dependencies via BFS on POM files.
    let all_deps = resolve_transitive(&direct_deps, resolver)?;

    eprintln!(
        "  Resolved {} dependencies ({} direct, {} transitive)",
        all_deps.len(),
        direct_deps.len(),
        all_deps.len().saturating_sub(direct_deps.len())
    );

    // 5. Partition deps into "already locked" vs "needs download" before any
    //    parallel work, so we don't read the lockfile under contention.
    //
    //    Then download the "needs download" set in parallel (across deps),
    //    while each dep also parallelizes its per-target downloads (nested
    //    rayon is fine).
    let mut new_dep_locks: Vec<DependencyLock> = Vec::with_capacity(all_deps.len());
    let mut needs_download: Vec<(usize, &ResolvedMavenDep)> = Vec::new();

    for (idx, dep) in all_deps.iter().enumerate() {
        let maven_coord = dep.key();
        let dep_classifier = &dep.classifier;
        let already_locked = lockfile.dependencies.iter().find(|d| {
            d.name == dep.name
                && matches!(&d.source, DepSource::Maven { version: v, maven: m, classifier: c, .. }
                    if v == &dep.version && m == &maven_coord && c == dep_classifier)
        });

        if let Some(existing) = already_locked {
            // Reserve a slot — we'll fill the same index for needs_download
            // entries after the parallel work completes.
            new_dep_locks.push(existing.clone());
            eprintln!("  Resolving {} {}...", dep.name, dep.version);
            eprintln!("    (already up to date)");
        } else {
            // Placeholder; replaced below.
            new_dep_locks.push(placeholder_lock(dep, &maven_coord));
            needs_download.push((idx, dep));
        }
    }

    // Download into the shared Maven cache so `konvoy build` can reuse the
    // verified klibs without re-downloading.
    let cache_root: PathBuf = crate::plugin::maven_cache_root()?;

    // Hosts all per-(dep, target) bars. Bars are added in stable order
    // (`needs_download` order × `KNOWN_TARGETS` order) BEFORE any parallel
    // work starts so the on-screen rows stay fixed regardless of which
    // download finishes first.
    let multi = konvoy_util::progress::new_multi_progress();
    let known_targets = konvoy_targets::KNOWN_TARGETS;

    // Build labels once, then reuse them to compute the max width AND to
    // construct the bars — avoids formatting each prefix twice.
    let labels: Vec<Vec<String>> = needs_download
        .iter()
        .map(|(_, dep)| known_targets.iter().map(|&t| dep.bar_label(t)).collect())
        .collect();
    let prefix_width = labels
        .iter()
        .flat_map(|row| row.iter().map(String::len))
        .max()
        .unwrap_or(0);

    let dep_bars: Vec<Vec<konvoy_util::progress::DownloadBar>> = labels
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|label| konvoy_util::progress::add_download_bar(&multi, label, prefix_width))
                .collect()
        })
        .collect();

    // Download deps in parallel; per-target downloads are also parallel within
    // each. `zip` pairs each dep with its pre-allocated bar row, removing any
    // index-based fallback.
    let download_results: Vec<Result<(usize, DependencyLock), EngineError>> = needs_download
        .par_iter()
        .zip(dep_bars.par_iter())
        .map(|((idx, dep), bars)| {
            let lock = download_dep(dep, &cache_root, bars, resolver)?;
            Ok((*idx, lock))
        })
        .collect();

    // Bars remain on screen at their final state after each is abandoned
    // (inside `DownloadBar::mark_success` / `mark_failure`); the
    // `MultiProgress` is dropped at end of scope which releases the draw
    // region without clearing the rendered content. Indicatif leaves the
    // cursor at the end of the last bar's row, so emit one trailing newline
    // before any subsequent `eprintln!` so the caller's status line doesn't
    // get concatenated onto the bar row.
    if !needs_download.is_empty() {
        eprintln!();
    }

    // Apply results in original order so the lockfile output is deterministic
    // (parallel downloads finish in arbitrary order, but the on-disk layout
    // remains stable).
    for result in download_results {
        let (idx, lock) = result?;
        if let Some(slot) = new_dep_locks.get_mut(idx) {
            *slot = lock;
        }
    }

    // After applying parallel results, every placeholder should have been
    // replaced with a real lock entry carrying a non-empty source_hash. Use
    // a debug_assert so a future bug (e.g. an index miscount or a skipped
    // download) surfaces immediately in test/debug builds without changing
    // release-time behavior.
    for lock in &new_dep_locks {
        debug_assert!(
            !lock.source_hash.is_empty(),
            "placeholder lock for {} not replaced",
            lock.name
        );
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

impl ResolvedMavenDep {
    /// Render the `"groupId:artifactId"` key used in lockfile entries and BFS maps.
    fn key(&self) -> String {
        format!("{}:{}", self.group_id, self.artifact_id)
    }

    /// Render the human-readable label shown on a download progress bar.
    fn bar_label(&self, target: konvoy_targets::Target) -> String {
        format!("{} {} [{}]", self.name, self.version, target)
    }
}

// ---------------------------------------------------------------------------
// Transitive resolution (BFS on artifact metadata)
// ---------------------------------------------------------------------------

/// A BFS queue entry: `(group_id, artifact_id, version, requirer_name, ancestor_path)`.
///
/// `requirer_name` is `None` for direct deps and `Some(parent_name)` for
/// transitive ones; `ancestor_path` carries the `group:artifact` chain
/// from a direct dep down to here for cycle detection.
type BfsEntry = (String, String, String, Option<String>, Vec<String>);

/// State accumulated during the BFS traversal of Maven transitive dependencies.
struct BfsState {
    user_versions: HashMap<String, String>,
    resolved: HashMap<String, ResolvedMavenDep>,
    metadata_cache: HashMap<String, ArtifactMetadata>,
    required_by_map: HashMap<String, BTreeSet<String>>,
    queue: VecDeque<BfsEntry>,
}

/// Collect the deduped union of **direct** Maven dependencies across the build
/// graph — the root manifest plus every path-dependency — surfacing a version
/// conflict when two projects declare the same `group:artifact` at different
/// versions.
///
/// Each `(label, manifest)` pair is a project; `label` names it in conflict
/// messages (e.g. `"konvoy.toml"` for the root, or a path-dep's name). A
/// path-dep's `[dependencies]` must be resolved and pinned just like the root's,
/// because each project's own compile links the klibs it declares — the
/// `[dependencies]` analogue of the path-dep `[plugins]` fix (#293).
///
/// Unlike compiler plugins (applied per-compilation, so multiple versions are
/// benign), Maven libraries are linked into a shared artifact graph, so a
/// cross-graph version clash is a hard error here — consistent with how the
/// transitive resolver already surfaces conflicts rather than auto-picking.
///
/// # Errors
/// Returns [`EngineError::MavenVersionConflict`] on a cross-project version
/// clash, or a coordinate-parse error for a malformed `maven` value.
fn collect_graph_direct_maven_deps<'a>(
    projects: impl IntoIterator<Item = (&'a str, &'a Manifest)>,
) -> Result<Vec<ResolvedMavenDep>, EngineError> {
    // Keyed by `group:artifact` (the dedup/conflict key), value carries the dep
    // plus the label of the project that first declared it (for the message).
    let mut by_key: BTreeMap<String, (ResolvedMavenDep, String)> = BTreeMap::new();

    for (label, manifest) in projects {
        for (dep_name, spec) in &manifest.dependencies {
            let Some((maven, version)) = spec.as_maven_coord() else {
                continue; // path dep or incomplete — not a Maven dep
            };
            let (group_id, artifact_id) = crate::common::split_maven_coordinate(maven)?;
            let dep = ResolvedMavenDep {
                name: dep_name.clone(),
                group_id: group_id.to_owned(),
                artifact_id: artifact_id.to_owned(),
                version: version.to_owned(),
                required_by: Vec::new(),
                classifier: None,
            };
            let key = dep.key();

            match by_key.get(&key) {
                Some((existing, existing_label)) if existing.version != dep.version => {
                    return Err(maven_version_conflict(
                        &key,
                        artifact_id,
                        existing_label,
                        &existing.version,
                        label,
                        &dep.version,
                    ));
                }
                Some(_) => { /* identical declaration — dedup */ }
                None => {
                    by_key.insert(key, (dep, label.to_owned()));
                }
            }
        }
    }

    Ok(by_key.into_values().map(|(dep, _)| dep).collect())
}

/// Order two Maven version strings well enough to pick the higher one for a
/// conflict hint. Compares the dotted-numeric core (`1.10.0` > `1.9.0`, and a
/// missing trailing segment counts as `0` so `1.0` == `1.0.0`); a version with a
/// pre-release suffix sorts BELOW the same core (`1.0.0-beta` < `1.0.0`), and two
/// suffixes compare lexicographically. Not a full semver implementation — it
/// only has to tolerate arbitrary Maven version strings and produce a sensible
/// hint.
fn compare_maven_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    // Split a version into its numeric core (segments before the first `-`) and
    // an optional pre-release suffix.
    fn split_core(v: &str) -> (&str, Option<&str>) {
        match v.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (v, None),
        }
    }

    let (a_core, a_pre) = split_core(a);
    let (b_core, b_pre) = split_core(b);

    let mut a_parts = a_core.split('.');
    let mut b_parts = b_core.split('.');
    loop {
        match (a_parts.next(), b_parts.next()) {
            (None, None) => break,
            // Pad a missing segment with 0 so `1.0` and `1.0.0` compare equal,
            // and `1.0.1` > `1.0`.
            (a_seg, b_seg) => {
                let x = a_seg.unwrap_or("0");
                let y = b_seg.unwrap_or("0");
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    _ => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }

    // Equal numeric core: a pre-release suffix is LESS than no suffix; two
    // suffixes compare lexicographically.
    match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

/// Build a [`EngineError::MavenVersionConflict`] naming the two clashing
/// requirers and suggesting the higher version to pin.
///
/// Single source of the conflict message format + hint policy, shared by the
/// graph-direct collector ([`collect_graph_direct_maven_deps`]) and the
/// transitive resolver ([`check_version_conflict`]) so they can't drift.
fn maven_version_conflict(
    dep_key: &str,
    base_artifact_id: &str,
    existing_requirer: &str,
    existing_version: &str,
    current_requirer: &str,
    current_version: &str,
) -> EngineError {
    let hint_version = if compare_maven_versions(current_version, existing_version)
        == std::cmp::Ordering::Greater
    {
        current_version
    } else {
        existing_version
    };
    EngineError::MavenVersionConflict {
        maven: dep_key.to_owned(),
        details: format!(
            "  {existing_requirer} requires {existing_version}\n  {current_requirer} requires {current_version}"
        ),
        hint_name: base_artifact_id.replace('.', "-"),
        hint_version: hint_version.to_owned(),
    }
}

/// The name of the dep that requires the child currently being recorded.
///
/// A queue entry's `requirer` field carries the processing dep's OWN name (set
/// when it was enqueued); for the seed (direct) deps it is `None`, so we fall
/// back to looking the processing dep up in `resolved` by its `group:artifact`,
/// and finally to `"unknown"`. Shared by both the already-resolved and new-dep
/// branches so they attribute `required_by` identically.
fn child_requirer_name(
    requirer: &Option<String>,
    group_id: &str,
    artifact_id: &str,
    resolved: &HashMap<String, ResolvedMavenDep>,
) -> String {
    requirer
        .clone()
        .or_else(|| {
            resolved
                .get(&format!("{group_id}:{artifact_id}"))
                .map(|d| d.name.clone())
        })
        .unwrap_or_else(|| "unknown".to_owned())
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

    Err(maven_version_conflict(
        dep_key,
        base_artifact_id,
        existing_requirer,
        &existing.version,
        current_requirer,
        resolved_version,
    ))
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

/// Finalize the `required_by` fields: every dep with recorded requirers gets the
/// accumulated set (including a dep that is BOTH declared direct and pulled
/// transitively — see the body). A purely-direct dep has no recorded requirers
/// and stays empty.
fn finalize_required_by(
    resolved: &mut HashMap<String, ResolvedMavenDep>,
    required_by_map: &HashMap<String, BTreeSet<String>>,
) {
    // A dep's `required_by` must record EVERY transitive requirer, even when the
    // dep is ALSO declared direct somewhere in the graph. The per-project Maven
    // closure (`build::project_maven_closure`) reconstructs a path-dep's
    // transitive set SOLELY from `required_by`, so emptying it for graph-direct
    // deps would make a transitive-that's-also-direct unreachable for a sibling
    // path-dep that needs it — a standalone-vs-as-a-dependency parity break. A
    // purely-direct dep simply has no `required_by_map` entry and stays empty.
    for (key, dep) in resolved.iter_mut() {
        if let Some(requirers) = required_by_map.get(key) {
            dep.required_by = requirers.iter().cloned().collect();
        }
    }
}

/// Resolve the full transitive closure of Maven dependencies via a
/// level-parallel BFS on artifact metadata (`.module` JSON first, POM XML
/// as fallback).
///
/// Each BFS level is processed in three phases:
/// 1. Drain the queue and fold per-entry `required_by` bookkeeping (no I/O).
/// 2. Dedupe + parallel-fetch metadata for unique `(group, artifact, version)`
///    tuples not already in the in-memory cache (rayon `par_iter`).
/// 3. Sequentially process each entry against the now-populated cache,
///    pushing children onto the queue for the next level. Cycle detection
///    uses the per-entry ancestor path.
///
/// Uses the first known target ([`konvoy_targets::Target::LinuxX64`]) to
/// discover transitive deps — the dependency graph is identical across
/// targets, only the klib differs.
///
/// After the graph is resolved, [`discover_cinterop_deps`] scans the
/// metadata cache for cinterop `.klib` files and adds them as separate
/// entries with `classifier` set.
///
/// # Errors
///
/// Returns an error if metadata cannot be fetched or parsed, a version
/// conflict is detected (see [`check_version_conflict`]), or a dependency
/// cycle is found.
fn resolve_transitive(
    direct_deps: &[ResolvedMavenDep],
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<Vec<ResolvedMavenDep>, EngineError> {
    // Use the first known target for metadata fetching — the transitive graph is
    // identical across targets since every Kotlin/Native artifact publishes
    // the same set of dependencies in each per-target variant. The enum
    // discriminant gives us a probe target without any fallible lookup.
    let probe_target = konvoy_targets::Target::LinuxX64;
    let maven_suffix = probe_target.to_maven_suffix();

    let mut state = BfsState {
        user_versions: HashMap::new(),
        resolved: HashMap::new(),
        metadata_cache: HashMap::new(),
        required_by_map: HashMap::new(),
        queue: VecDeque::new(),
    };

    // Build user-specified version map and seed the BFS queue.
    for dep in direct_deps {
        let key = dep.key();
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

    // Level-based BFS: drain the queue into a Vec, fetch all metadata for
    // that level in parallel, then process children sequentially so the
    // graph mutations remain race-free.
    while !state.queue.is_empty() {
        // Drain the current level into a Vec paired with its per-entry cache
        // keys. The key derivation is identical for prefetch and post-fetch
        // passes; compute it once.
        let level: Vec<(BfsEntry, String)> = state
            .queue
            .drain(..)
            .map(|entry| {
                let cache_key = format!("{}:{}-{}:{}", entry.0, entry.1, maven_suffix, entry.2);
                (entry, cache_key)
            })
            .collect();

        // Fold pre-fetch bookkeeping (required_by entries from `requirer`)
        // into the map before doing any I/O — these only depend on `requirer`
        // and `(group_id, artifact_id)`, not on the fetched metadata.
        for ((group_id, artifact_id, _version, requirer, _path), _cache_key) in &level {
            let key = format!("{group_id}:{artifact_id}");
            if let Some(req) = requirer {
                state
                    .required_by_map
                    .entry(key)
                    .or_default()
                    .insert(req.clone());
            }
        }

        // Collect unique tuples that need fetching and are not already in the
        // metadata cache. Multiple entries in the queue may point at the same
        // artifact via different paths.
        let mut to_fetch: Vec<(String, String, String, String)> = Vec::with_capacity(level.len());
        let mut seen_keys: BTreeSet<String> = BTreeSet::new();
        for ((group_id, artifact_id, version, _requirer, _path), cache_key) in &level {
            if state.metadata_cache.contains_key(cache_key) || !seen_keys.insert(cache_key.clone())
            {
                continue;
            }
            to_fetch.push((
                group_id.clone(),
                format!("{artifact_id}-{maven_suffix}"),
                version.clone(),
                cache_key.clone(),
            ));
        }

        // Parallel fetch — no shared mutable state.
        let fetched: Vec<Result<(String, ArtifactMetadata), EngineError>> = to_fetch
            .par_iter()
            .map(|(group_id, per_target_artifact_id, version, cache_key)| {
                let metadata = resolver
                    .fetch_artifact_metadata(
                        group_id,
                        per_target_artifact_id,
                        version,
                        maven_suffix,
                    )
                    .map_err(EngineError::Util)?;
                Ok((cache_key.clone(), metadata))
            })
            .collect();

        // Merge fetched metadata into the cache (sequential, no contention).
        for result in fetched {
            let (cache_key, metadata) = result?;
            state.metadata_cache.insert(cache_key, metadata);
        }

        // Process this level's entries sequentially using the now-populated cache.
        for ((group_id, artifact_id, _version, requirer, path), cache_key) in level {
            // Every entry was either cached or just fetched above; a miss
            // here means a logic bug. Surface as a typed error so tests catch
            // future refactors that break this invariant.
            let metadata = match state.metadata_cache.get(&cache_key) {
                Some(m) => m.clone(),
                None => {
                    return Err(EngineError::InternalInvariantViolated {
                        context: format!("BFS prefetch missed metadata for {cache_key}"),
                    });
                }
            };

            for meta_dep in &metadata.dependencies {
                if is_filtered_dependency(&meta_dep.group_id, &meta_dep.artifact_id) {
                    continue;
                }
                if meta_dep.version.is_empty() {
                    continue;
                }

                let base_artifact_id = strip_target_suffix(&meta_dep.artifact_id, maven_suffix);
                let dep_key = format!("{}:{}", meta_dep.group_id, base_artifact_id);

                let resolved_version = state
                    .user_versions
                    .get(&dep_key)
                    .cloned()
                    .unwrap_or_else(|| meta_dep.version.clone());

                // Already resolved — check for version conflict, then record the
                // requiring edge and skip re-queueing. The requirer is THIS entry
                // (the dep whose metadata we're reading), not this entry's own
                // requirer — matching the new-dep branch below. Recording it even
                // for an already-resolved (e.g. direct-seeded) child is what lets
                // `build::project_maven_closure` reach a transitive that is also
                // declared direct somewhere in the graph.
                if let Some(existing) = state.resolved.get(&dep_key) {
                    check_version_conflict(
                        existing,
                        &resolved_version,
                        &dep_key,
                        &base_artifact_id,
                        requirer.as_deref(),
                    )?;
                    let existing_name = existing.name.clone();
                    let parent_name =
                        child_requirer_name(&requirer, &group_id, &artifact_id, &state.resolved);
                    // Skip a self-reference (a dep whose POM lists itself): this
                    // branch has no cycle check, and `A required_by [A]` is a
                    // confusing lockfile entry that the old wipe used to hide.
                    if parent_name != existing_name {
                        state
                            .required_by_map
                            .entry(dep_key)
                            .or_default()
                            .insert(parent_name);
                    }
                    continue;
                }

                let dep_name = base_artifact_id.clone();
                let parent_name =
                    child_requirer_name(&requirer, &group_id, &artifact_id, &state.resolved);

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
    }

    finalize_required_by(&mut state.resolved, &state.required_by_map);

    let cinterop_deps =
        discover_cinterop_deps(&state.resolved, &state.metadata_cache, maven_suffix);

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
// Helpers
// ---------------------------------------------------------------------------

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

    /// Create a nested `lib` path-dependency under `project/<name>` so
    /// `update` (which now resolves the whole graph) can find it. Nested rather
    /// than a `/tmp` sibling so parallel tests don't collide on a shared path.
    fn add_nested_lib_dep(project: &std::path::Path, name: &str, kotlin: &str) {
        let dep = project.join(name);
        fs::create_dir_all(dep.join("src")).unwrap();
        fs::write(
            dep.join("konvoy.toml"),
            format!("[package]\nname = \"{name}\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"{kotlin}\"\n"),
        )
        .unwrap();
        fs::write(dep.join("src/lib.kt"), "package dep\nfun f() = 1\n").unwrap();
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
my-utils = { path = "my-utils" }
"#,
        );
        add_nested_lib_dep(project.path(), "my-utils", "2.1.0");
        // Write an initial lockfile with a path dep.
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-utils".to_owned(),
            source: DepSource::Path {
                path: "my-utils".to_owned(),
            },
            source_hash: "abcdef".to_owned(),
        });
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        let result = update(project.path(), crate::common::test_resolver(false, false)).unwrap();
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
my-utils = { path = "my-utils" }
"#,
        );
        add_nested_lib_dep(project.path(), "my-utils", "2.1.0");
        // Pre-populate lockfile with a path dep.
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-utils".to_owned(),
            source: DepSource::Path {
                path: "my-utils".to_owned(),
            },
            source_hash: "path-hash".to_owned(),
        });
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        let result = update(project.path(), crate::common::test_resolver(false, false)).unwrap();
        assert_eq!(result.updated_count, 0);

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert_eq!(reparsed.dependencies.len(), 1);
        let dep = &reparsed.dependencies[0];
        assert_eq!(dep.name, "my-utils");
        match &dep.source {
            DepSource::Path { path } => assert_eq!(path, "my-utils"),
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

        let result = update(project.path(), crate::common::test_resolver(false, false)).unwrap();
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

        let result = update(project.path(), crate::common::test_resolver(false, false)).unwrap();
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
        let result = resolve_transitive(&[], crate::common::test_resolver(false, false)).unwrap();
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
        let result = update(project.path(), crate::common::test_resolver(false, false)).unwrap();
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

        let result = update(project.path(), crate::common::test_resolver(false, false)).unwrap();
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
        let result = resolve_transitive(&[], crate::common::test_resolver(false, false)).unwrap();
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
    fn resolved_maven_dep_key_is_group_colon_artifact() {
        let dep = ResolvedMavenDep {
            name: "kotlinx-coroutines".to_owned(),
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "kotlinx-coroutines-core".to_owned(),
            version: "1.8.0".to_owned(),
            required_by: Vec::new(),
            classifier: None,
        };
        assert_eq!(dep.key(), "org.jetbrains.kotlinx:kotlinx-coroutines-core");
    }

    #[test]
    fn finalize_required_by_records_every_transitive_requirer() {
        // A transitive dep that is ALSO declared direct must STILL record its
        // transitive requirers — the per-project Maven closure relies on
        // `required_by` to reach it (emptying it broke standalone-vs-as-dep
        // parity; see the comment in `finalize_required_by`).
        let direct_and_transitive = ResolvedMavenDep {
            name: "atomicfu".to_owned(),
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "atomicfu".to_owned(),
            version: "0.23.1".to_owned(),
            required_by: Vec::new(),
            classifier: None,
        };

        let mut resolved = HashMap::new();
        resolved.insert(direct_and_transitive.key(), direct_and_transitive.clone());

        let mut required_by_map: HashMap<String, BTreeSet<String>> = HashMap::new();
        // atomicfu is pulled in by coroutines AND declared direct by the root.
        required_by_map.insert(
            direct_and_transitive.key(),
            BTreeSet::from(["kotlinx-coroutines-core".to_owned()]),
        );

        finalize_required_by(&mut resolved, &required_by_map);

        assert_eq!(
            resolved
                .get(&direct_and_transitive.key())
                .expect("dep still present")
                .required_by,
            vec!["kotlinx-coroutines-core".to_owned()],
            "a direct-AND-transitive dep must keep its transitive requirer so the closure can reach it"
        );
    }

    #[test]
    fn finalize_required_by_skips_deps_with_no_map_entry() {
        // A dep absent from `required_by_map` is left untouched.
        let dep = ResolvedMavenDep {
            name: "isolated".to_owned(),
            group_id: "org.example".to_owned(),
            artifact_id: "isolated".to_owned(),
            version: "1.0.0".to_owned(),
            required_by: vec!["pre-existing".to_owned()],
            classifier: None,
        };
        let mut resolved = HashMap::new();
        resolved.insert(dep.key(), dep.clone());
        let required_by_map: HashMap<String, BTreeSet<String>> = HashMap::new();

        finalize_required_by(&mut resolved, &required_by_map);

        assert_eq!(
            resolved
                .get(&dep.key())
                .expect("dep still present")
                .required_by,
            vec!["pre-existing".to_owned()],
            "dep with no map entry should be untouched"
        );
    }

    #[test]
    fn resolved_maven_dep_key_ignores_version_and_classifier() {
        // `key()` is a group:artifact identity — version and classifier must
        // not influence it (they're separate fields in lockfile entries).
        let a = ResolvedMavenDep {
            name: "atomicfu".to_owned(),
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "atomicfu".to_owned(),
            version: "0.23.1".to_owned(),
            required_by: Vec::new(),
            classifier: None,
        };
        let b = ResolvedMavenDep {
            version: "0.24.0".to_owned(),
            classifier: Some("cinterop-interop".to_owned()),
            ..a.clone()
        };
        assert_eq!(a.key(), b.key());
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
        let result = update(project.path(), crate::common::test_resolver(false, false)).unwrap();
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

    // -------------------------------------------------------------------
    // placeholder_lock
    // -------------------------------------------------------------------

    #[test]
    fn placeholder_lock_produces_maven_source_with_empty_targets_and_hash() {
        // The placeholder is overwritten after download_dep completes, but its
        // shape still matters: if the post-download replacement is ever skipped
        // (e.g. an index miscount) the placeholder propagates into the
        // lockfile. The empty `source_hash` is what the debug_assert in
        // `update()` keys on to catch that bug.
        let dep = ResolvedMavenDep {
            name: "kotlinx-coroutines".to_owned(),
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "kotlinx-coroutines-core".to_owned(),
            version: "1.8.0".to_owned(),
            required_by: vec!["root".to_owned()],
            classifier: Some("cinterop-interop".to_owned()),
        };
        let coord = "org.jetbrains.kotlinx:kotlinx-coroutines-core";
        let lock = placeholder_lock(&dep, coord);

        assert_eq!(lock.name, "kotlinx-coroutines");
        assert!(
            lock.source_hash.is_empty(),
            "placeholder source_hash must be empty so the debug_assert in update() fires if it leaks"
        );
        match &lock.source {
            DepSource::Maven {
                version,
                maven,
                targets,
                required_by,
                classifier,
            } => {
                assert_eq!(version, "1.8.0");
                assert_eq!(maven, coord);
                assert!(
                    targets.is_empty(),
                    "placeholder targets must be empty — they are filled by download_dep"
                );
                assert_eq!(required_by, &vec!["root".to_owned()]);
                assert_eq!(classifier.as_deref(), Some("cinterop-interop"));
            }
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    #[test]
    fn placeholder_lock_no_classifier_for_main_klib() {
        // The classifier flows through unchanged; for a regular dep it stays None.
        let dep = ResolvedMavenDep {
            name: "atomicfu".to_owned(),
            group_id: "org.jetbrains.kotlinx".to_owned(),
            artifact_id: "atomicfu".to_owned(),
            version: "0.23.1".to_owned(),
            required_by: Vec::new(),
            classifier: None,
        };
        let lock = placeholder_lock(&dep, "org.jetbrains.kotlinx:atomicfu");
        match &lock.source {
            DepSource::Maven { classifier, .. } => assert!(classifier.is_none()),
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }

    // -- graph-wide direct Maven dep collection (#293 analogue for [dependencies]) --

    fn manifest_with_deps(deps: &[(&str, &str, &str)]) -> Manifest {
        let mut toml = String::from(
            "[package]\nname = \"p\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"2.2.0\"\n\n[dependencies]\n",
        );
        for (name, maven, version) in deps {
            toml.push_str(&format!(
                "{name} = {{ maven = \"{maven}\", version = \"{version}\" }}\n"
            ));
        }
        Manifest::from_str(&toml, "konvoy.toml").unwrap()
    }

    #[test]
    fn collect_graph_direct_maven_deps_unions_and_dedupes() {
        // The root and a path-dep both declare coroutines (identical → dedup); the
        // dep also declares datetime. The union has both, exactly once each.
        let root = manifest_with_deps(&[(
            "coroutines",
            "org.jetbrains.kotlinx:kotlinx-coroutines-core",
            "1.8.0",
        )]);
        let dep = manifest_with_deps(&[
            (
                "coroutines",
                "org.jetbrains.kotlinx:kotlinx-coroutines-core",
                "1.8.0",
            ),
            (
                "datetime",
                "org.jetbrains.kotlinx:kotlinx-datetime",
                "0.6.0",
            ),
        ]);

        let union =
            collect_graph_direct_maven_deps([("konvoy.toml", &root), ("models", &dep)]).unwrap();
        let ids: Vec<String> = union
            .iter()
            .map(|d| format!("{}:{}", d.key(), d.version))
            .collect();
        assert_eq!(union.len(), 2, "identical declarations must dedup to one");
        assert!(ids.contains(&"org.jetbrains.kotlinx:kotlinx-coroutines-core:1.8.0".to_owned()));
        assert!(ids.contains(&"org.jetbrains.kotlinx:kotlinx-datetime:0.6.0".to_owned()));
    }

    #[test]
    fn collect_graph_direct_maven_deps_surfaces_cross_project_conflict() {
        // Libraries are linked, so the same artifact at two versions across the
        // graph is a conflict to surface — not distinct pins (unlike plugins).
        let root = manifest_with_deps(&[(
            "coroutines",
            "org.jetbrains.kotlinx:kotlinx-coroutines-core",
            "1.8.0",
        )]);
        let dep = manifest_with_deps(&[(
            "coroutines",
            "org.jetbrains.kotlinx:kotlinx-coroutines-core",
            "1.7.3",
        )]);

        let err = collect_graph_direct_maven_deps([("konvoy.toml", &root), ("models", &dep)])
            .unwrap_err();
        match err {
            EngineError::MavenVersionConflict { details, maven, .. } => {
                assert_eq!(maven, "org.jetbrains.kotlinx:kotlinx-coroutines-core");
                assert!(details.contains("1.8.0"), "details: {details}");
                assert!(
                    details.contains("models requires 1.7.3"),
                    "conflict must name the declaring project: {details}"
                );
            }
            other => panic!("expected MavenVersionConflict, got {other:?}"),
        }
    }

    #[test]
    fn child_requirer_name_prefers_requirer_then_resolved_then_unknown() {
        let mut resolved: HashMap<String, ResolvedMavenDep> = HashMap::new();
        resolved.insert(
            "org.jetbrains.kotlinx:kotlinx-coroutines-core".to_owned(),
            ResolvedMavenDep {
                name: "coroutines".to_owned(),
                group_id: "org.jetbrains.kotlinx".to_owned(),
                artifact_id: "kotlinx-coroutines-core".to_owned(),
                version: "1.8.0".to_owned(),
                required_by: Vec::new(),
                classifier: None,
            },
        );

        // requirer is Some → used directly (the cached processing-dep name).
        assert_eq!(
            child_requirer_name(
                &Some("coroutines".to_owned()),
                "org.jetbrains.kotlinx",
                "kotlinx-coroutines-core",
                &resolved,
            ),
            "coroutines"
        );
        // requirer None (a seed/direct dep) → fall back to the resolved entry's name.
        assert_eq!(
            child_requirer_name(
                &None,
                "org.jetbrains.kotlinx",
                "kotlinx-coroutines-core",
                &resolved,
            ),
            "coroutines"
        );
        // requirer None and the processing dep isn't resolved → "unknown".
        assert_eq!(
            child_requirer_name(&None, "com.example", "missing", &resolved),
            "unknown"
        );
        // requirer Some takes PRECEDENCE over the resolved-name fallback, even
        // when they differ.
        assert_eq!(
            child_requirer_name(
                &Some("explicit".to_owned()),
                "org.jetbrains.kotlinx",
                "kotlinx-coroutines-core",
                &resolved,
            ),
            "explicit"
        );
    }

    #[test]
    fn compare_maven_versions_is_numeric_not_lexicographic() {
        use std::cmp::Ordering;
        // The bug this guards: a string compare makes "1.10.0" < "1.9.0".
        assert_eq!(compare_maven_versions("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(compare_maven_versions("1.9.0", "1.10.0"), Ordering::Less);
        assert_eq!(compare_maven_versions("2.0.0", "2.0.0"), Ordering::Equal);
        assert_eq!(compare_maven_versions("1.0.1", "1.0"), Ordering::Greater);
        assert_eq!(compare_maven_versions("0.6.1", "0.6.0"), Ordering::Greater);
    }

    #[test]
    fn compare_maven_versions_handles_segments_and_prerelease() {
        use std::cmp::Ordering;
        // Missing trailing segments count as 0.
        assert_eq!(compare_maven_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_maven_versions("1.0.0", "1.0"), Ordering::Equal);
        // A pre-release suffix sorts BELOW the same numeric core (the bug the
        // old lexicographic fallback got backwards: "1.0.0-beta" > "1.0.0").
        assert_eq!(
            compare_maven_versions("1.0.0-beta", "1.0.0"),
            Ordering::Less
        );
        assert_eq!(
            compare_maven_versions("1.0.0", "1.0.0-RC1"),
            Ordering::Greater
        );
        // Two suffixes compare lexicographically; numeric core still dominates.
        assert_eq!(
            compare_maven_versions("1.0.0-rc1", "1.0.0-rc2"),
            Ordering::Less
        );
        assert_eq!(
            compare_maven_versions("2.0.0-beta", "1.0.0"),
            Ordering::Greater
        );
    }

    #[test]
    fn compare_maven_versions_reflexive_and_antisymmetric() {
        use std::cmp::Ordering;
        for v in ["1.0.0", "1.2", "0.6.1", "1.0.0-beta", "2.0"] {
            assert_eq!(compare_maven_versions(v, v), Ordering::Equal);
        }
        // a < b  <=>  b > a, across mixed numeric/suffix cases.
        for (a, b) in [
            ("1.9.0", "1.10.0"),
            ("1.0.0-beta", "1.0.0"),
            ("0.6.0", "0.6.1"),
        ] {
            assert_eq!(compare_maven_versions(a, b), Ordering::Less);
            assert_eq!(compare_maven_versions(b, a), Ordering::Greater);
        }
    }

    #[test]
    fn compare_maven_versions_non_numeric_segment_falls_back_lexically() {
        use std::cmp::Ordering;
        // A non-numeric core segment can't be parsed → byte comparison for that
        // segment (best effort; the function is hint-only).
        assert_eq!(compare_maven_versions("1.x.0", "1.y.0"), Ordering::Less);
        // Trailing zeros don't change ordering.
        assert_eq!(compare_maven_versions("1", "1.0.0.0"), Ordering::Equal);
    }

    #[test]
    fn maven_version_conflict_munges_hint_name_and_picks_existing_when_higher() {
        // hint_name turns dots into dashes (the konvoy.toml key convention); when
        // the existing pin is the higher version, it's the suggested one.
        let err = maven_version_conflict(
            "org.jetbrains.kotlinx:atomicfu",
            "org.jetbrains.kotlinx.atomicfu",
            "konvoy.toml",
            "0.24.0",
            "models",
            "0.23.1",
        );
        match err {
            EngineError::MavenVersionConflict {
                maven,
                hint_name,
                hint_version,
                details,
            } => {
                assert_eq!(maven, "org.jetbrains.kotlinx:atomicfu");
                assert_eq!(hint_name, "org-jetbrains-kotlinx-atomicfu");
                assert_eq!(hint_version, "0.24.0", "existing is higher → suggested");
                assert!(details.contains("konvoy.toml requires 0.24.0"));
                assert!(details.contains("models requires 0.23.1"));
            }
            other => panic!("expected MavenVersionConflict, got {other:?}"),
        }
    }

    #[test]
    fn finalize_required_by_accumulates_and_sorts_multiple_requirers() {
        // A transitive dep pulled in by two parents records BOTH, sorted (the
        // map is a BTreeSet, so order is deterministic).
        let dep = ResolvedMavenDep {
            name: "shared".to_owned(),
            group_id: "g".to_owned(),
            artifact_id: "shared".to_owned(),
            version: "1.0".to_owned(),
            required_by: Vec::new(),
            classifier: None,
        };
        let mut resolved = HashMap::new();
        resolved.insert(dep.key(), dep.clone());
        let mut required_by_map: HashMap<String, BTreeSet<String>> = HashMap::new();
        required_by_map.insert(
            dep.key(),
            BTreeSet::from(["zed".to_owned(), "alpha".to_owned()]),
        );

        finalize_required_by(&mut resolved, &required_by_map);
        assert_eq!(
            resolved.get(&dep.key()).unwrap().required_by,
            vec!["alpha".to_owned(), "zed".to_owned()],
            "requirers are deterministic (sorted)"
        );
    }

    #[test]
    fn collect_graph_direct_maven_deps_three_projects_dedup_keeps_first_declarer() {
        // root, libA, libB. root and libA declare coroutines under DIFFERENT
        // konvoy keys (same coord+version) — dedup keeps the FIRST declarer's
        // name (root). libB adds a distinct dep.
        let root = manifest_with_deps(&[(
            "coroutines",
            "org.jetbrains.kotlinx:kotlinx-coroutines-core",
            "1.8.0",
        )]);
        let lib_a = manifest_with_deps(&[(
            "my-coroutines",
            "org.jetbrains.kotlinx:kotlinx-coroutines-core",
            "1.8.0",
        )]);
        let lib_b = manifest_with_deps(&[(
            "datetime",
            "org.jetbrains.kotlinx:kotlinx-datetime",
            "0.6.0",
        )]);

        let union = collect_graph_direct_maven_deps([
            ("konvoy.toml", &root),
            ("libA", &lib_a),
            ("libB", &lib_b),
        ])
        .unwrap();
        let by_key: std::collections::BTreeMap<String, &str> =
            union.iter().map(|d| (d.key(), d.name.as_str())).collect();
        assert_eq!(union.len(), 2, "the shared coordinate dedups to one entry");
        assert_eq!(
            by_key.get("org.jetbrains.kotlinx:kotlinx-coroutines-core"),
            Some(&"coroutines"),
            "the first declarer's (root's) konvoy key wins"
        );
        assert!(by_key.contains_key("org.jetbrains.kotlinx:kotlinx-datetime"));
    }

    #[test]
    fn maven_version_conflict_hint_suggests_the_higher_version() {
        let err = maven_version_conflict(
            "org.jetbrains.kotlinx:kotlinx-coroutines-core",
            "kotlinx-coroutines-core",
            "konvoy.toml",
            "1.9.0",
            "models",
            "1.10.0",
        );
        match err {
            EngineError::MavenVersionConflict {
                hint_version,
                details,
                ..
            } => {
                assert_eq!(
                    hint_version, "1.10.0",
                    "hint must suggest the higher version"
                );
                assert!(details.contains("konvoy.toml requires 1.9.0"));
                assert!(details.contains("models requires 1.10.0"));
            }
            other => panic!("expected MavenVersionConflict, got {other:?}"),
        }
    }

    #[test]
    fn update_prunes_stale_maven_when_graph_has_none() {
        // A lockfile pins a Maven dep that the manifest no longer declares (and
        // there are no Maven deps anywhere). update() must drop the stale pin
        // while keeping path-dep locks.
        let project = make_project(
            "[package]\nname = \"app\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n\n[dependencies]\nmy-utils = { path = \"my-utils\" }\n",
        );
        add_nested_lib_dep(project.path(), "my-utils", "2.1.0");
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-utils".to_owned(),
            source: DepSource::Path {
                path: "my-utils".to_owned(),
            },
            source_hash: "h".to_owned(),
        });
        lockfile.dependencies.push(DependencyLock {
            name: "kotlinx-datetime".to_owned(),
            source: DepSource::Maven {
                version: "0.6.0".to_owned(),
                maven: "org.jetbrains.kotlinx:kotlinx-datetime".to_owned(),
                targets: std::collections::BTreeMap::new(),
                required_by: Vec::new(),
                classifier: None,
            },
            source_hash: "stale".to_owned(),
        });
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        update(project.path(), crate::common::test_resolver(false, false)).unwrap();

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert!(
            !reparsed.has_maven_entry("kotlinx-datetime"),
            "stale Maven pin must be pruned"
        );
        assert_eq!(reparsed.dependencies.len(), 1, "the path dep must be kept");
        assert_eq!(reparsed.dependencies[0].name, "my-utils");
    }

    #[test]
    fn collect_graph_direct_maven_deps_ignores_path_deps() {
        // A path-dep entry in a manifest is not a Maven dep and must be skipped.
        let root = Manifest::from_str(
            "[package]\nname = \"p\"\nkind = \"bin\"\n\n[toolchain]\nkotlin = \"2.2.0\"\n\n[dependencies]\nmodels = { path = \"../models\" }\n",
            "konvoy.toml",
        )
        .unwrap();
        let union = collect_graph_direct_maven_deps([("konvoy.toml", &root)]).unwrap();
        assert!(union.is_empty(), "path deps are not Maven deps");
    }
}
