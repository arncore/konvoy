//! The `konvoy update` command: resolve Maven deps and populate lockfile hashes.
//!
//! For each dependency in `konvoy.toml` that specifies a `version` (i.e. a Maven
//! dependency rather than a path dependency), this module:
//!
//! 1. Looks up the library in the curated index.
//! 2. For every known Kotlin/Native target, downloads the klib from Maven Central
//!    and computes its SHA-256 hash.
//! 3. Stores all per-target hashes in the lockfile under `DepSource::Maven`.
//!
//! The command is idempotent: if the lockfile already contains a Maven entry at
//! the same version, the download is skipped.

use std::collections::BTreeMap;
use std::path::Path;

use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use konvoy_config::lockfile::{DepSource, DependencyLock, Lockfile};
use konvoy_config::manifest::Manifest;
use konvoy_util::maven::MAVEN_CENTRAL;

use crate::error::EngineError;
use crate::library;

/// Result of an update operation.
#[derive(Debug)]
pub struct UpdateResult {
    /// Number of Maven dependencies that were resolved.
    pub updated_count: usize,
}

/// Resolve Maven dependency versions and update `konvoy.lock` with per-target hashes.
///
/// # Errors
/// Returns an error if a library is not in the curated index, a download fails,
/// or the lockfile cannot be written.
pub fn update(project_root: &Path) -> Result<UpdateResult, EngineError> {
    // 1. Read konvoy.toml and konvoy.lock.
    let manifest_path = project_root.join("konvoy.toml");
    let manifest = Manifest::from_path(&manifest_path)?;

    let lockfile_path = project_root.join("konvoy.lock");
    let mut lockfile = Lockfile::from_path(&lockfile_path)?;

    // 2. Collect Maven deps (those with `version` set).
    let maven_deps: Vec<_> = manifest
        .dependencies
        .iter()
        .filter(|(_, spec)| spec.version.is_some())
        .collect();

    if maven_deps.is_empty() {
        lockfile.write_to(&lockfile_path)?;
        return Ok(UpdateResult { updated_count: 0 });
    }

    let mut new_dep_locks = Vec::new();

    for (dep_name, dep_spec) in &maven_deps {
        // safe: filtered above
        let Some(version) = dep_spec.version.as_ref() else {
            continue;
        };

        // Look up in curated index.
        let descriptor = library::lookup(dep_name)?.ok_or_else(|| EngineError::UnknownLibrary {
            name: (*dep_name).clone(),
            available: library::available_library_names().unwrap_or_else(|_| "none".to_owned()),
        })?;

        eprintln!("  Resolving {} {}...", dep_name, version);

        // Check if lockfile already has this dep at this version -- skip if so.
        let already_locked = lockfile.dependencies.iter().any(|d| {
            d.name == **dep_name
                && matches!(&d.source, DepSource::Maven { version: v, .. } if v == version)
        });
        if already_locked {
            eprintln!("    (already up to date)");
            // Preserve existing entry.
            if let Some(lock) = lockfile.dependencies.iter().find(|d| d.name == **dep_name) {
                new_dep_locks.push(lock.clone());
            }
            continue;
        }

        // For each known target, download and hash (in parallel).
        let known_targets = konvoy_targets::known_targets();

        // Create a temp directory for downloads using std::env::temp_dir.
        let pid = std::process::id();
        let tmp_base = std::env::temp_dir().join(format!("konvoy-update-{pid}"));
        konvoy_util::fs::ensure_dir(&tmp_base)?;

        let target_results: Vec<Result<(String, String), EngineError>> = known_targets
            .par_iter()
            .map(|target_name| {
                let target = target_name
                    .parse::<konvoy_targets::Target>()
                    .map_err(EngineError::Target)?;
                let coord = library::resolve_coordinate(&descriptor, version, &target)?;
                let url = coord.to_url(MAVEN_CENTRAL);

                // Download to a temp file, hash it, delete.
                let tmp_file = tmp_base.join(coord.filename());

                let result = konvoy_util::artifact::ensure_artifact(
                    &url,
                    &tmp_file,
                    None, // no expected hash -- we are discovering it
                    &format!("{dep_name}:{target_name}"),
                    version,
                )
                .map_err(|e| match e {
                    konvoy_util::error::UtilError::Download { message } => {
                        EngineError::LibraryDownloadFailed {
                            name: (*dep_name).clone(),
                            url: url.clone(),
                            message,
                        }
                    }
                    other => EngineError::Util(other),
                })?;

                // Clean up the downloaded file.
                let _ = std::fs::remove_file(&tmp_file);

                Ok(((*target_name).to_owned(), result.sha256))
            })
            .collect();

        // Collect results and print hashes.
        let mut targets_map: BTreeMap<String, String> = BTreeMap::new();
        for result in target_results {
            let (target_name, sha256) = result?;
            let display_hash = sha256.get(..16).unwrap_or(&sha256);
            eprintln!("    {}: {}...", target_name, display_hash);
            targets_map.insert(target_name, sha256);
        }

        // Clean up the temp directory.
        let _ = std::fs::remove_dir_all(&tmp_base);

        // Build the maven coordinate template (with {target} placeholder) for the lockfile.
        let maven_coordinate = descriptor.maven.replace("{version}", version);

        // Compute a deterministic source_hash from the target hashes.
        let hash_input: String = targets_map
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join("\n");
        let source_hash = konvoy_util::hash::sha256_bytes(hash_input.as_bytes());

        new_dep_locks.push(DependencyLock {
            name: (*dep_name).clone(),
            source: DepSource::Maven {
                version: version.clone(),
                maven_coordinate,
                targets: targets_map,
            },
            source_hash,
        });
    }

    // 3. Merge: preserve existing path deps and toolchain/plugin info,
    //    replace all Maven deps with newly resolved ones.
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

    // 4. Write updated lockfile.
    lockfile.write_to(&lockfile_path)?;

    Ok(UpdateResult {
        updated_count: maven_deps.len(),
    })
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
    fn update_rejects_unknown_library() {
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
unknown-lib = { version = "1.0.0" }
"#,
        );

        let result = update(project.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown library"),
            "error should mention 'unknown library', got: {err}"
        );
        assert!(
            err.contains("kotlinx-coroutines"),
            "error should list available libraries, got: {err}"
        );
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
    fn update_idempotent_skip_existing_maven_dep() {
        let project = make_project(
            r#"
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
kotlinx-coroutines = { version = "1.8.0" }
"#,
        );
        // Pre-populate lockfile with a Maven dep at the same version.
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
                maven_coordinate:
                    "org.jetbrains.kotlinx:kotlinx-coroutines-core-{target}:1.8.0:klib".to_owned(),
                targets: targets.clone(),
            },
            source_hash: "existing-hash".to_owned(),
        });
        lockfile
            .write_to(&project.path().join("konvoy.lock"))
            .unwrap();

        // Running update should be idempotent (skip the already-resolved dep).
        let result = update(project.path()).unwrap();
        assert_eq!(result.updated_count, 1); // still counts as 1 maven dep found

        let reparsed = Lockfile::from_path(&project.path().join("konvoy.lock")).unwrap();
        assert_eq!(reparsed.dependencies.len(), 1);
        let dep = &reparsed.dependencies[0];
        assert_eq!(dep.name, "kotlinx-coroutines");
        // Should still have the old hash since we skipped re-download.
        assert_eq!(dep.source_hash, "existing-hash");
        match &dep.source {
            DepSource::Maven { version, .. } => assert_eq!(version, "1.8.0"),
            other => panic!("expected Maven source, got: {other:?}"),
        }
    }
}
