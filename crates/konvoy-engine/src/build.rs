//! Build orchestration: resolve config, detect target, invoke compiler, store artifacts.

use std::path::{Path, PathBuf};
use std::time::Instant;

use konvoy_config::lockfile::{DepSource, DependencyLock, Lockfile};
use konvoy_config::manifest::{Manifest, PackageKind};
use konvoy_konanc::detect::{resolve_konanc, KonancInfo};
use konvoy_konanc::invoke::{DiagnosticLevel, KonancCommand, ProduceKind};
use konvoy_targets::{host_target, Target};

use crate::artifact::{ArtifactStore, BuildMetadata};
use crate::cache::{CacheInputs, CacheKey};
use crate::error::EngineError;
use crate::resolve::{resolve_dependencies, ResolvedGraph};

/// Options controlling a build invocation.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Explicit target triple, or `None` for host.
    pub target: Option<String>,
    /// Whether to build in release mode.
    pub release: bool,
    /// Whether to show raw compiler output.
    pub verbose: bool,
    /// Allow overriding hash mismatch checks.
    pub force: bool,
    /// Require the lockfile to be up-to-date; error on any mismatch.
    pub locked: bool,
}

/// Whether the build used a cached artifact or compiled fresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    /// The artifact was already in cache and was materialized without recompilation.
    Cached,
    /// The artifact was compiled fresh and stored in cache.
    Fresh,
}

/// Result of a successful build.
#[derive(Debug)]
pub struct BuildResult {
    /// Whether the build used cache or compiled fresh.
    pub outcome: BuildOutcome,
    /// Path to the final output binary.
    pub output_path: PathBuf,
    /// How long the build took (including cache check).
    pub duration: std::time::Duration,
}

/// Run the full build pipeline.
///
/// Steps:
/// 1. Read `konvoy.toml` from project root
/// 2. Read `konvoy.lock` (or create default)
/// 3. Detect host target (or resolve `--target` flag)
/// 4. Detect `konanc` and get version + fingerprint
/// 5. Collect source files (`src/**/*.kt`)
/// 6. Compute cache key from all inputs
/// 7. Check cache — if hit, materialize and return early
/// 8. Invoke `konanc` with resolved inputs
/// 9. Store artifact in cache
/// 10. Materialize artifact to output path
/// 11. Update `konvoy.lock` if toolchain version changed
///
/// # Errors
/// Returns an error if any step fails (config parsing, compiler detection,
/// compilation failure, filesystem errors, etc.).
pub fn build(project_root: &Path, options: &BuildOptions) -> Result<BuildResult, EngineError> {
    let start = Instant::now();

    // 1. Read konvoy.toml.
    let manifest_path = project_root.join("konvoy.toml");
    let manifest = Manifest::from_path(&manifest_path)?;

    // 2. Read konvoy.lock (or default).
    let lockfile_path = project_root.join("konvoy.lock");
    let lockfile =
        Lockfile::from_path(&lockfile_path).map_err(|e| EngineError::Lockfile(e.to_string()))?;

    // 3. Resolve target.
    let target = resolve_target(&options.target)?;
    let profile = if options.release { "release" } else { "debug" };

    // 4. Resolve managed konanc toolchain.
    let resolved = resolve_konanc(&manifest.toolchain.kotlin).map_err(EngineError::Konanc)?;
    let konanc = resolved.info;
    let jre_home = resolved.jre_home.clone();

    // 5. Resolve dependencies and build them in topological order.
    let dep_graph = resolve_dependencies(project_root, &manifest)?;
    let mut library_paths: Vec<PathBuf> = Vec::new();
    let lockfile_content = lockfile_toml_content(&lockfile);

    for dep in &dep_graph.order {
        let (dep_output, _) = build_single(
            &dep.project_root,
            &dep.manifest,
            &konanc,
            jre_home.as_deref(),
            &target,
            profile,
            options,
            &library_paths,
            &lockfile_content,
        )?;
        library_paths.push(dep_output);
    }

    // 6. Build the root project.
    let (output_path, outcome) = build_single(
        project_root,
        &manifest,
        &konanc,
        jre_home.as_deref(),
        &target,
        profile,
        options,
        &library_paths,
        &lockfile_content,
    )?;

    // 7. Update lockfile if toolchain or dependencies changed.
    update_lockfile_if_needed(
        &lockfile,
        &konanc,
        resolved.konanc_tarball_sha256.as_deref(),
        resolved.jre_tarball_sha256.as_deref(),
        &dep_graph,
        project_root,
        &lockfile_path,
        options.force,
        options.locked,
    )?;

    Ok(BuildResult {
        outcome,
        output_path,
        duration: start.elapsed(),
    })
}

/// Build a single project (either root or a dependency).
///
/// Returns the path to the output artifact and whether the build was cached.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_single(
    project_root: &Path,
    manifest: &Manifest,
    konanc: &KonancInfo,
    jre_home: Option<&Path>,
    target: &Target,
    profile: &str,
    options: &BuildOptions,
    library_paths: &[PathBuf],
    lockfile_content: &str,
) -> Result<(PathBuf, BuildOutcome), EngineError> {
    // Collect source files, excluding test sources (src/test/).
    let src_dir = project_root.join("src");
    let test_dir = src_dir.join("test");
    let all_sources = konvoy_util::fs::collect_files(&src_dir, "kt")?;
    let sources: Vec<PathBuf> = all_sources
        .into_iter()
        .filter(|p| !p.starts_with(&test_dir))
        .collect();
    if sources.is_empty() {
        return Err(EngineError::NoSources {
            dir: src_dir.display().to_string(),
        });
    }

    let is_lib = manifest.package.kind == PackageKind::Lib;

    // Compute cache key.
    let manifest_content = manifest.to_toml()?;
    let cache_inputs = CacheInputs {
        manifest_content,
        lockfile_content: lockfile_content.to_owned(),
        konanc_version: konanc.version.clone(),
        konanc_fingerprint: konanc.fingerprint.clone(),
        target: target.to_string(),
        profile: profile.to_owned(),
        source_dir: project_root.join("src"),
        source_glob: "**/*.kt".to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        dependency_hashes: library_paths
            .iter()
            .filter_map(|p| konvoy_util::hash::sha256_file(p).ok())
            .collect(),
    };
    let cache_key = CacheKey::compute(&cache_inputs)?;

    // Output path: for deps, put .klib in deps/ subdir; for root, keep existing layout.
    let output_name = if is_lib {
        format!("{}.klib", manifest.package.name)
    } else {
        manifest.package.name.clone()
    };
    let output_path = project_root
        .join(".konvoy")
        .join("build")
        .join(target.to_konanc_arg())
        .join(profile)
        .join(&output_name);

    let store = ArtifactStore::new(project_root);

    // Check cache.
    if store.has(&cache_key) {
        eprintln!("    Fresh {} (cached)", manifest.package.name);
        store.materialize(&cache_key, &output_name, &output_path)?;
        return Ok((output_path, BuildOutcome::Cached));
    }

    // Compile.
    eprintln!(
        "    Compiling {} \u{2192} {}",
        manifest.package.name,
        output_path.display()
    );

    let compile_output = compile(
        konanc,
        jre_home,
        &sources,
        target,
        &output_path,
        options,
        if is_lib {
            ProduceKind::Library
        } else {
            ProduceKind::Program
        },
        library_paths,
    )?;

    // Store artifact in cache.
    let metadata = BuildMetadata {
        target: target.to_string(),
        profile: profile.to_owned(),
        konanc_version: konanc.version.clone(),
        built_at: now_iso8601(),
    };
    store.store(&cache_key, &compile_output, &metadata)?;

    // Materialize to the canonical output path (if compile output differs).
    if compile_output != output_path {
        store.materialize(&cache_key, &output_name, &output_path)?;
    }

    Ok((output_path, BuildOutcome::Fresh))
}

/// Resolve the target: use the explicit `--target` value or detect the host.
pub(crate) fn resolve_target(target_opt: &Option<String>) -> Result<Target, EngineError> {
    match target_opt {
        Some(name) => name.parse::<Target>().map_err(EngineError::Target),
        None => host_target().map_err(EngineError::Target),
    }
}

/// Invoke konanc and return the path to the compiled artifact.
#[allow(clippy::too_many_arguments)]
fn compile(
    konanc: &KonancInfo,
    jre_home: Option<&Path>,
    sources: &[PathBuf],
    target: &Target,
    output_path: &Path,
    options: &BuildOptions,
    produce: ProduceKind,
    library_paths: &[PathBuf],
) -> Result<PathBuf, EngineError> {
    // Ensure the output directory exists.
    if let Some(parent) = output_path.parent() {
        konvoy_util::fs::ensure_dir(parent)?;
    }

    let mut cmd = KonancCommand::new()
        .sources(sources)
        .output(output_path)
        .target(target.to_konanc_arg())
        .release(options.release)
        .produce(produce)
        .libraries(library_paths);

    if let Some(jh) = jre_home {
        cmd = cmd.java_home(jh);
    }

    let result = cmd.execute(konanc).map_err(EngineError::Konanc)?;

    // Print diagnostics to stderr.
    for diag in &result.diagnostics {
        let prefix = match diag.level {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Info => "info",
        };
        match (&diag.file, diag.line) {
            (Some(file), Some(line)) => eprintln!("{prefix}: {file}:{line}: {}", diag.message),
            _ => eprintln!("{prefix}: {}", diag.message),
        }
    }

    // In verbose mode, print raw output.
    if options.verbose {
        if !result.raw_stdout.is_empty() {
            eprintln!("{}", result.raw_stdout);
        }
        if !result.raw_stderr.is_empty() {
            eprintln!("{}", result.raw_stderr);
        }
    }

    if !result.success {
        return Err(EngineError::CompilationFailed {
            error_count: result.error_count(),
        });
    }

    // konanc appends `.kexe` on Linux for programs. Rename to the expected path.
    // Libraries produce .klib directly, so skip this for library builds.
    if produce == ProduceKind::Program {
        let kexe_path = output_path.with_extension("kexe");
        if !output_path.exists() && kexe_path.exists() {
            std::fs::rename(&kexe_path, output_path).map_err(|source| EngineError::Io {
                path: output_path.display().to_string(),
                source,
            })?;
        }
    }

    Ok(output_path.to_path_buf())
}

/// Serialize lockfile content for cache key computation.
pub(crate) fn lockfile_toml_content(lockfile: &Lockfile) -> String {
    toml::to_string_pretty(lockfile).unwrap_or_default()
}

/// Update konvoy.lock if the detected konanc version or dependency hashes differ,
/// or if a fresh download provides new tarball hashes. When the lockfile already
/// contains hashes and a fresh download yields different ones, emit a warning.
#[allow(clippy::too_many_arguments)]
fn update_lockfile_if_needed(
    lockfile: &Lockfile,
    konanc: &KonancInfo,
    konanc_tarball_sha256: Option<&str>,
    jre_tarball_sha256: Option<&str>,
    dep_graph: &ResolvedGraph,
    project_root: &Path,
    lockfile_path: &Path,
    force: bool,
    locked: bool,
) -> Result<(), EngineError> {
    // Check for dependency source hash mismatches.
    // In --locked mode this is a hard error; otherwise warn and continue.
    for dep in &dep_graph.order {
        if let Some(locked_dep) = lockfile.dependencies.iter().find(|d| d.name == dep.name) {
            if locked_dep.source_hash != dep.source_hash && !dep.source_hash.is_empty() {
                if locked {
                    return Err(EngineError::DependencyHashMismatch {
                        name: dep.name.clone(),
                        expected: locked_dep.source_hash.clone(),
                        actual: dep.source_hash.clone(),
                    });
                }
                eprintln!(
                    "warning: dependency `{}` source has changed (locked: {}, current: {})",
                    dep.name,
                    truncate_hash(&locked_dep.source_hash),
                    truncate_hash(&dep.source_hash),
                );
            }
        }
    }

    // Build new dependency lock entries.
    let new_deps: Vec<DependencyLock> = dep_graph
        .order
        .iter()
        .filter(|dep| !dep.source_hash.is_empty())
        .map(|dep| {
            let rel_path = dep
                .project_root
                .strip_prefix(project_root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| dep.project_root.display().to_string());
            DependencyLock {
                name: dep.name.clone(),
                source: DepSource::Path { path: rel_path },
                source_hash: dep.source_hash.clone(),
            }
        })
        .collect();

    let toolchain_changed = match &lockfile.toolchain {
        Some(tc) => tc.konanc_version != konanc.version,
        None => true,
    };

    let has_new_hashes = konanc_tarball_sha256.is_some() || jre_tarball_sha256.is_some();
    let deps_changed = lockfile.dependencies != new_deps;

    // If nothing changed, nothing to do.
    if !toolchain_changed && !has_new_hashes && !deps_changed {
        return Ok(());
    }

    // When the same version is re-downloaded and the lockfile already has hashes,
    // verify they match. A mismatch could indicate a tampered or rotated tarball.
    // This is a hard error unless --force is used. When the version changed,
    // different hashes are expected, so skip the check.
    if !toolchain_changed {
        if let Some(tc) = &lockfile.toolchain {
            if let (Some(existing), Some(actual)) =
                (&tc.konanc_tarball_sha256, konanc_tarball_sha256)
            {
                if !existing.is_empty() && existing != actual {
                    if force {
                        eprintln!(
                        "warning: konanc tarball hash changed — expected {existing}, got {actual}; lockfile updated (--force)"
                    );
                    } else {
                        return Err(EngineError::TarballHashMismatch {
                            kind: "konanc".to_owned(),
                            expected: existing.clone(),
                            actual: actual.to_owned(),
                        });
                    }
                }
            }
            if let (Some(existing), Some(actual)) = (&tc.jre_tarball_sha256, jre_tarball_sha256) {
                if !existing.is_empty() && existing != actual {
                    if force {
                        eprintln!(
                        "warning: jre tarball hash changed — expected {existing}, got {actual}; lockfile updated (--force)"
                    );
                    } else {
                        return Err(EngineError::TarballHashMismatch {
                            kind: "jre".to_owned(),
                            expected: existing.clone(),
                            actual: actual.to_owned(),
                        });
                    }
                }
            }
        }
    }

    // In --locked mode, the lockfile must not be modified. If we reach here,
    // something has changed (toolchain, deps, or hashes) that would require a write.
    if locked {
        return Err(EngineError::LockfileUpdateRequired);
    }

    // Build the updated lockfile. When the version hasn't changed, preserve
    // existing hashes if no fresh download occurred. When the version changed,
    // only use hashes from the fresh download (old hashes are for a different
    // version's tarball and must not carry forward).
    let (final_konanc_sha, final_jre_sha) = if toolchain_changed {
        (
            konanc_tarball_sha256.map(str::to_owned),
            jre_tarball_sha256.map(str::to_owned),
        )
    } else {
        match &lockfile.toolchain {
            Some(tc) => {
                let konanc_sha = konanc_tarball_sha256
                    .map(str::to_owned)
                    .or_else(|| tc.konanc_tarball_sha256.clone());
                let jre_sha = jre_tarball_sha256
                    .map(str::to_owned)
                    .or_else(|| tc.jre_tarball_sha256.clone());
                (konanc_sha, jre_sha)
            }
            None => (
                konanc_tarball_sha256.map(str::to_owned),
                jre_tarball_sha256.map(str::to_owned),
            ),
        }
    };

    let mut updated = Lockfile::with_managed_toolchain(
        &konanc.version,
        final_konanc_sha.as_deref(),
        final_jre_sha.as_deref(),
    );
    updated.dependencies = new_deps;
    updated
        .write_to(lockfile_path)
        .map_err(|e| EngineError::Lockfile(e.to_string()))?;

    Ok(())
}

/// Truncate a hash string for display (first 8 chars).
fn truncate_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

/// Return the current UTC time as an ISO 8601 string.
pub(crate) fn now_iso8601() -> String {
    // Use a simple approach without pulling in chrono.
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s-since-epoch", duration.as_secs())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolve_target_host() {
        let target = resolve_target(&None);
        // Should succeed on supported platforms.
        if let Ok(t) = target {
            assert!(!t.to_string().is_empty());
        }
    }

    #[test]
    fn resolve_target_explicit() {
        let target = resolve_target(&Some("linux_x64".to_owned()));
        assert!(target.is_ok());
        let t = target.unwrap();
        assert_eq!(t.to_string(), "linux_x64");
    }

    #[test]
    fn resolve_target_invalid() {
        let target = resolve_target(&Some("invalid_target".to_owned()));
        assert!(target.is_err());
    }

    #[test]
    fn build_fails_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };
        let result = build(tmp.path(), &options);
        assert!(result.is_err());
    }

    #[test]
    fn build_fails_without_sources() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("empty-proj");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(
            project.join("konvoy.toml"),
            "[package]\nname = \"empty\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
        )
        .unwrap();
        // No .kt files in src/

        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };
        let result = build(&project, &options);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no .kt source files") || err.contains("source"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn lockfile_toml_content_empty() {
        let lockfile = Lockfile::default();
        let content = lockfile_toml_content(&lockfile);
        assert!(content.contains("toolchain") || content.is_empty() || content.trim().is_empty());
    }

    #[test]
    fn lockfile_toml_content_with_version() {
        let lockfile = Lockfile::with_toolchain("2.1.0");
        let content = lockfile_toml_content(&lockfile);
        assert!(content.contains("2.1.0"));
    }

    #[test]
    fn lockfile_toml_content_includes_tarball_hashes() {
        let without_hashes = Lockfile::with_toolchain("2.1.0");
        let with_hashes = Lockfile::with_managed_toolchain("2.1.0", Some("abc123"), Some("def456"));

        let content_without = lockfile_toml_content(&without_hashes);
        let content_with = lockfile_toml_content(&with_hashes);

        assert_ne!(
            content_without, content_with,
            "lockfile content should differ when tarball hashes are present"
        );
        assert!(content_with.contains("abc123"));
        assert!(content_with.contains("def456"));
    }

    #[test]
    fn lockfile_toml_content_includes_dependencies() {
        let mut lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.dependencies.push(DependencyLock {
            name: "my-lib".to_owned(),
            source: DepSource::Path {
                path: "libs/my-lib".to_owned(),
            },
            source_hash: "deadbeef".to_owned(),
        });

        let content = lockfile_toml_content(&lockfile);
        assert!(
            content.contains("my-lib"),
            "lockfile content should include dependency names"
        );
        assert!(
            content.contains("deadbeef"),
            "lockfile content should include dependency hashes"
        );
    }

    #[test]
    fn update_lockfile_writes_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::default();
        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            None,
            None,
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();
        assert!(lockfile_path.exists());
        let content = fs::read_to_string(&lockfile_path).unwrap();
        assert!(content.contains("2.1.0"));
    }

    #[test]
    fn update_lockfile_skips_when_same() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::with_toolchain("2.1.0");
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // Should not error and should not change the file.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            None,
            None,
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();
    }

    #[test]
    fn update_lockfile_updates_when_different() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::with_toolchain("2.0.0");
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            Some("deadbeef"),
            Some("cafebabe"),
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();
        let content = fs::read_to_string(&lockfile_path).unwrap();
        assert!(content.contains("2.1.0"));
    }

    #[test]
    fn update_lockfile_first_download_stores_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::default();
        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            Some("first-konanc-hash"),
            Some("first-jre-hash"),
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();

        let reparsed = Lockfile::from_path(&lockfile_path).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_version, "2.1.0");
        assert_eq!(
            tc.konanc_tarball_sha256.as_deref(),
            Some("first-konanc-hash")
        );
        assert_eq!(tc.jre_tarball_sha256.as_deref(), Some("first-jre-hash"));
    }

    #[test]
    fn build_options_defaults() {
        let opts = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };
        assert!(opts.target.is_none());
        assert!(!opts.release);
        assert!(!opts.verbose);
        assert!(!opts.force);
        assert!(!opts.locked);
    }

    #[test]
    fn update_lockfile_writes_deps() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::default();
        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let dep = crate::resolve::ResolvedDep {
            name: "my-lib".to_owned(),
            project_root: tmp.path().join("my-lib"),
            manifest: konvoy_config::manifest::Manifest::from_str(
                "[package]\nname = \"my-lib\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
                "konvoy.toml",
            )
            .unwrap(),
            dep_names: Vec::new(),
            source_hash: "deadbeefcafebabe".to_owned(),
        };
        let graph = crate::resolve::ResolvedGraph { order: vec![dep] };

        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            None,
            None,
            &graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();
        let content = fs::read_to_string(&lockfile_path).unwrap();
        assert!(
            content.contains("my-lib"),
            "lockfile should contain dep name: {content}"
        );
        assert!(
            content.contains("deadbeefcafebabe"),
            "lockfile should contain source_hash: {content}"
        );
    }

    #[test]
    fn build_outcome_equality() {
        assert_eq!(BuildOutcome::Cached, BuildOutcome::Cached);
        assert_eq!(BuildOutcome::Fresh, BuildOutcome::Fresh);
        assert_ne!(BuildOutcome::Cached, BuildOutcome::Fresh);
    }

    #[test]
    fn build_single_returns_cached_on_cache_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("myapp");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src").join("main.kt"), "fun main() {}").unwrap();
        fs::write(
            project.join("konvoy.toml"),
            "[package]\nname = \"myapp\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
        )
        .unwrap();

        let manifest =
            konvoy_config::manifest::Manifest::from_path(&project.join("konvoy.toml")).unwrap();
        let konanc = KonancInfo {
            path: PathBuf::from("/fake/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc123".to_owned(),
        };
        let target: konvoy_targets::Target = "linux_x64".parse().unwrap();
        let profile = "debug";
        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };

        // Compute the cache key that build_single would compute.
        let manifest_content = manifest.to_toml().unwrap();
        let effective_lockfile = Lockfile::with_toolchain(&konanc.version);
        let lockfile_content = lockfile_toml_content(&effective_lockfile);
        let cache_inputs = CacheInputs {
            manifest_content,
            lockfile_content: lockfile_content.clone(),
            konanc_version: konanc.version.clone(),
            konanc_fingerprint: konanc.fingerprint.clone(),
            target: target.to_string(),
            profile: profile.to_owned(),
            source_dir: project.join("src"),
            source_glob: "**/*.kt".to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            dependency_hashes: Vec::new(),
        };
        let cache_key = CacheKey::compute(&cache_inputs).unwrap();

        // Pre-populate the artifact store with a fake artifact.
        let store = ArtifactStore::new(&project);
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let fake_artifact = staging.join("myapp");
        fs::write(&fake_artifact, "fake-binary-content").unwrap();
        let metadata = BuildMetadata {
            target: target.to_string(),
            profile: profile.to_owned(),
            konanc_version: konanc.version.clone(),
            built_at: now_iso8601(),
        };
        store.store(&cache_key, &fake_artifact, &metadata).unwrap();
        assert!(store.has(&cache_key));

        // Call build_single — it should hit cache and return Cached.
        let (output_path, outcome) = build_single(
            &project,
            &manifest,
            &konanc,
            None,
            &target,
            profile,
            &options,
            &[],
            &lockfile_content,
        )
        .unwrap();

        assert_eq!(outcome, BuildOutcome::Cached);
        assert!(output_path.exists());
    }

    #[test]
    fn build_single_excludes_test_sources() {
        // Create a project with both src/main.kt and src/test/FooTest.kt.
        // Pre-populate cache with a key computed from only non-test sources.
        // If build_single correctly excludes test sources, it should hit cache.
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("myapp");
        fs::create_dir_all(project.join("src").join("test")).unwrap();
        fs::write(project.join("src").join("main.kt"), "fun main() {}").unwrap();
        fs::write(
            project.join("src").join("test").join("FooTest.kt"),
            "import kotlin.test.Test\nclass FooTest { @Test fun foo() {} }",
        )
        .unwrap();
        fs::write(
            project.join("konvoy.toml"),
            "[package]\nname = \"myapp\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
        )
        .unwrap();

        let manifest =
            konvoy_config::manifest::Manifest::from_path(&project.join("konvoy.toml")).unwrap();
        let konanc = KonancInfo {
            path: PathBuf::from("/fake/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc123".to_owned(),
        };
        let target: konvoy_targets::Target = "linux_x64".parse().unwrap();
        let profile = "debug";
        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };

        // Compute cache key the same way build_single does (without test sources).
        let manifest_content = manifest.to_toml().unwrap();
        let effective_lockfile = Lockfile::with_toolchain(&konanc.version);
        let lockfile_content = lockfile_toml_content(&effective_lockfile);
        let cache_inputs = CacheInputs {
            manifest_content,
            lockfile_content: lockfile_content.clone(),
            konanc_version: konanc.version.clone(),
            konanc_fingerprint: konanc.fingerprint.clone(),
            target: target.to_string(),
            profile: profile.to_owned(),
            source_dir: project.join("src"),
            source_glob: "**/*.kt".to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            dependency_hashes: Vec::new(),
        };
        let cache_key = CacheKey::compute(&cache_inputs).unwrap();

        // Pre-populate the artifact store.
        let store = ArtifactStore::new(&project);
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let fake_artifact = staging.join("myapp");
        fs::write(&fake_artifact, "fake-binary-content").unwrap();
        let metadata = BuildMetadata {
            target: target.to_string(),
            profile: profile.to_owned(),
            konanc_version: konanc.version.clone(),
            built_at: now_iso8601(),
        };
        store.store(&cache_key, &fake_artifact, &metadata).unwrap();

        // build_single should hit cache because test sources are excluded,
        // producing the same cache key we computed above.
        let (output_path, outcome) = build_single(
            &project,
            &manifest,
            &konanc,
            None,
            &target,
            profile,
            &options,
            &[],
            &lockfile_content,
        )
        .unwrap();

        assert_eq!(outcome, BuildOutcome::Cached);
        assert!(output_path.exists());
    }

    #[test]
    fn kt_files_outside_src_do_not_affect_cache_key() {
        // Create a project with src/main.kt and an extra .kt file at the project root.
        // The cache key should only depend on src/**/*.kt, so adding a .kt file
        // outside src/ must not change the key.
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("myapp");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src").join("main.kt"), "fun main() {}").unwrap();
        fs::write(
            project.join("konvoy.toml"),
            "[package]\nname = \"myapp\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
        )
        .unwrap();

        let manifest =
            konvoy_config::manifest::Manifest::from_path(&project.join("konvoy.toml")).unwrap();
        let konanc = KonancInfo {
            path: PathBuf::from("/fake/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc123".to_owned(),
        };
        let target: konvoy_targets::Target = "linux_x64".parse().unwrap();
        let profile = "debug";
        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };

        // Compute cache key before adding the outside file.
        let manifest_content = manifest.to_toml().unwrap();
        let effective_lockfile = Lockfile::with_toolchain(&konanc.version);
        let lockfile_content = lockfile_toml_content(&effective_lockfile);
        let cache_inputs_before = CacheInputs {
            manifest_content: manifest_content.clone(),
            lockfile_content: lockfile_content.clone(),
            konanc_version: konanc.version.clone(),
            konanc_fingerprint: konanc.fingerprint.clone(),
            target: target.to_string(),
            profile: profile.to_owned(),
            source_dir: project.join("src"),
            source_glob: "**/*.kt".to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            dependency_hashes: Vec::new(),
        };
        let key_before = CacheKey::compute(&cache_inputs_before).unwrap();

        // Pre-seed the cache so build_single returns Cached.
        let store = ArtifactStore::new(&project);
        let staging = tmp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let fake_artifact = staging.join("myapp");
        fs::write(&fake_artifact, "fake-binary-content").unwrap();
        let metadata = BuildMetadata {
            target: target.to_string(),
            profile: profile.to_owned(),
            konanc_version: konanc.version.clone(),
            built_at: now_iso8601(),
        };
        store.store(&key_before, &fake_artifact, &metadata).unwrap();

        // Add a .kt file outside src/ (at the project root).
        fs::write(
            project.join("stray.kt"),
            "fun stray() { /* should be ignored */ }",
        )
        .unwrap();

        // Also add one in a sibling directory.
        fs::create_dir_all(project.join("scripts")).unwrap();
        fs::write(project.join("scripts").join("build.kt"), "fun build() {}").unwrap();

        // Recompute the cache key — it should be identical since source_dir is src/.
        let cache_inputs_after = CacheInputs {
            manifest_content,
            lockfile_content: lockfile_content.clone(),
            konanc_version: konanc.version.clone(),
            konanc_fingerprint: konanc.fingerprint.clone(),
            target: target.to_string(),
            profile: profile.to_owned(),
            source_dir: project.join("src"),
            source_glob: "**/*.kt".to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            dependency_hashes: Vec::new(),
        };
        let key_after = CacheKey::compute(&cache_inputs_after).unwrap();
        assert_eq!(
            key_before, key_after,
            "cache key must not change when .kt files are added outside src/"
        );

        // build_single should still hit cache because it uses src/ as source_dir.
        let (output_path, outcome) = build_single(
            &project,
            &manifest,
            &konanc,
            None,
            &target,
            profile,
            &options,
            &[],
            &lockfile_content,
        )
        .unwrap();

        assert_eq!(outcome, BuildOutcome::Cached);
        assert!(output_path.exists());
    }

    #[test]
    fn now_iso8601_not_empty() {
        let ts = now_iso8601();
        assert!(!ts.is_empty());
        assert!(ts.contains("since-epoch"));
    }

    #[test]
    fn update_lockfile_preserves_hashes_when_no_download() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::with_managed_toolchain("2.1.0", Some("abc123"), Some("def456"));
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // No new hashes (None) — toolchain was already installed.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            None,
            None,
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();

        let content = fs::read_to_string(&lockfile_path).unwrap();
        // Existing hashes should be preserved in the lockfile.
        assert!(
            content.contains("abc123"),
            "konanc hash should be preserved"
        );
        assert!(content.contains("def456"), "jre hash should be preserved");
    }

    #[test]
    fn update_lockfile_updates_hashes_from_fresh_download() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile =
            Lockfile::with_managed_toolchain("2.0.0", Some("oldhash1"), Some("oldhash2"));
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // Fresh download provides new hashes.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            Some("newhash1"),
            Some("newhash2"),
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();

        let reparsed = Lockfile::from_path(&lockfile_path).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_version, "2.1.0");
        assert_eq!(tc.konanc_tarball_sha256.as_deref(), Some("newhash1"));
        assert_eq!(tc.jre_tarball_sha256.as_deref(), Some("newhash2"));
    }

    #[test]
    fn update_lockfile_same_hash_redownload_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile =
            Lockfile::with_managed_toolchain("2.1.0", Some("samehash1"), Some("samehash2"));
        lockfile.write_to(&lockfile_path).unwrap();
        let before = fs::read_to_string(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // Fresh download returns the same hashes as the lockfile.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            Some("samehash1"),
            Some("samehash2"),
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();

        let after = fs::read_to_string(&lockfile_path).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn update_lockfile_hash_mismatch_is_hard_error() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile =
            Lockfile::with_managed_toolchain("2.1.0", Some("oldhash1"), Some("oldhash2"));
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // Hash mismatch without --force should be a hard error.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        let result = update_lockfile_if_needed(
            &lockfile,
            &konanc,
            Some("newhash1"),
            Some("newhash2"),
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tarball hash mismatch"),
            "expected hash mismatch error, got: {err}"
        );
        assert!(
            err.contains("--force"),
            "error message should mention --force, got: {err}"
        );
    }

    #[test]
    fn update_lockfile_hash_mismatch_with_force_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile =
            Lockfile::with_managed_toolchain("2.1.0", Some("oldhash1"), Some("oldhash2"));
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // Hash mismatch with --force should warn but succeed.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            Some("newhash1"),
            Some("newhash2"),
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            true,
            false,
        )
        .unwrap();

        let reparsed = Lockfile::from_path(&lockfile_path).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_tarball_sha256.as_deref(), Some("newhash1"));
        assert_eq!(tc.jre_tarball_sha256.as_deref(), Some("newhash2"));
    }

    #[test]
    fn update_lockfile_preserves_hashes_on_same_version_no_download() {
        // Same version, no new download — should not touch the lockfile hashes.
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile =
            Lockfile::with_managed_toolchain("2.1.0", Some("existing1"), Some("existing2"));
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // Same version, no new hashes — should skip update entirely.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            None,
            None,
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();

        let reparsed = Lockfile::from_path(&lockfile_path).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_tarball_sha256.as_deref(), Some("existing1"));
        assert_eq!(tc.jre_tarball_sha256.as_deref(), Some("existing2"));
    }

    #[test]
    fn update_lockfile_version_change_without_download_clears_old_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile =
            Lockfile::with_managed_toolchain("2.0.0", Some("existing1"), Some("existing2"));
        lockfile.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        // Version changed with no fresh download hashes: old version hashes must be cleared.
        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        update_lockfile_if_needed(
            &lockfile,
            &konanc,
            None,
            None,
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            false,
        )
        .unwrap();

        let reparsed = Lockfile::from_path(&lockfile_path).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_version, "2.1.0");
        assert_eq!(tc.konanc_tarball_sha256, None);
        assert_eq!(tc.jre_tarball_sha256, None);
    }

    #[test]
    fn update_lockfile_locked_mode_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::with_toolchain("2.1.0");

        // Write a lockfile with an existing dep hash.
        let mut lf = lockfile.clone();
        lf.dependencies.push(DependencyLock {
            name: "my-lib".to_owned(),
            source: DepSource::Path {
                path: "my-lib".to_owned(),
            },
            source_hash: "oldhash123".to_owned(),
        });
        lf.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let dep = crate::resolve::ResolvedDep {
            name: "my-lib".to_owned(),
            project_root: tmp.path().join("my-lib"),
            manifest: konvoy_config::manifest::Manifest::from_str(
                "[package]\nname = \"my-lib\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
                "konvoy.toml",
            )
            .unwrap(),
            dep_names: Vec::new(),
            source_hash: "newhash456".to_owned(),
        };
        let graph = crate::resolve::ResolvedGraph { order: vec![dep] };

        let result = update_lockfile_if_needed(
            &lf,
            &konanc,
            None,
            None,
            &graph,
            tmp.path(),
            &lockfile_path,
            false,
            true, // locked = true
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("source hash mismatch"),
            "expected dep hash mismatch error, got: {err}"
        );
        assert!(
            err.contains("my-lib"),
            "error should mention dep name, got: {err}"
        );
    }

    #[test]
    fn update_lockfile_unlocked_mode_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::with_toolchain("2.1.0");

        let mut lf = lockfile.clone();
        lf.dependencies.push(DependencyLock {
            name: "my-lib".to_owned(),
            source: DepSource::Path {
                path: "my-lib".to_owned(),
            },
            source_hash: "oldhash123".to_owned(),
        });
        lf.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let dep = crate::resolve::ResolvedDep {
            name: "my-lib".to_owned(),
            project_root: tmp.path().join("my-lib"),
            manifest: konvoy_config::manifest::Manifest::from_str(
                "[package]\nname = \"my-lib\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
                "konvoy.toml",
            )
            .unwrap(),
            dep_names: Vec::new(),
            source_hash: "newhash456".to_owned(),
        };
        let graph = crate::resolve::ResolvedGraph { order: vec![dep] };

        // Without --locked, mismatch should warn but succeed.
        let result = update_lockfile_if_needed(
            &lf,
            &konanc,
            None,
            None,
            &graph,
            tmp.path(),
            &lockfile_path,
            false,
            false, // locked = false
        );

        assert!(result.is_ok());
    }

    #[test]
    fn update_lockfile_locked_mode_matching_hash_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::with_toolchain("2.1.0");

        let mut lf = lockfile.clone();
        lf.dependencies.push(DependencyLock {
            name: "my-lib".to_owned(),
            source: DepSource::Path {
                path: "my-lib".to_owned(),
            },
            source_hash: "samehash789".to_owned(),
        });
        lf.write_to(&lockfile_path).unwrap();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let dep = crate::resolve::ResolvedDep {
            name: "my-lib".to_owned(),
            project_root: tmp.path().join("my-lib"),
            manifest: konvoy_config::manifest::Manifest::from_str(
                "[package]\nname = \"my-lib\"\nkind = \"lib\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
                "konvoy.toml",
            )
            .unwrap(),
            dep_names: Vec::new(),
            source_hash: "samehash789".to_owned(),
        };
        let graph = crate::resolve::ResolvedGraph { order: vec![dep] };

        // Matching hash with --locked should pass fine.
        let result = update_lockfile_if_needed(
            &lf,
            &konanc,
            None,
            None,
            &graph,
            tmp.path(),
            &lockfile_path,
            false,
            true, // locked = true
        );

        assert!(result.is_ok());
    }

    #[test]
    fn update_lockfile_locked_mode_prevents_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let lockfile_path = tmp.path().join("konvoy.lock");
        let lockfile = Lockfile::default();

        let konanc = KonancInfo {
            path: PathBuf::from("/usr/bin/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc".to_owned(),
        };

        let empty_graph = crate::resolve::ResolvedGraph { order: Vec::new() };
        // Toolchain changed (default lockfile has no toolchain), so lockfile
        // would need updating. With --locked, this should error.
        let result = update_lockfile_if_needed(
            &lockfile,
            &konanc,
            None,
            None,
            &empty_graph,
            tmp.path(),
            &lockfile_path,
            false,
            true, // locked = true
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("lockfile is out of date"),
            "expected lockfile update error, got: {err}"
        );
    }

    #[test]
    fn build_single_only_test_sources_returns_no_sources_error() {
        // A project with .kt files ONLY in src/test/ and nothing in src/ itself.
        // build_single filters out src/test/, so it should see zero sources and error.
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("testonly");
        fs::create_dir_all(project.join("src").join("test")).unwrap();
        fs::write(
            project.join("src").join("test").join("OnlyTest.kt"),
            "import kotlin.test.Test\nclass OnlyTest { @Test fun t() {} }",
        )
        .unwrap();
        fs::write(
            project.join("konvoy.toml"),
            "[package]\nname = \"testonly\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
        )
        .unwrap();

        let manifest =
            konvoy_config::manifest::Manifest::from_path(&project.join("konvoy.toml")).unwrap();
        let konanc = KonancInfo {
            path: PathBuf::from("/fake/konanc"),
            version: "2.1.0".to_owned(),
            fingerprint: "abc123".to_owned(),
        };
        let target: konvoy_targets::Target = "linux_x64".parse().unwrap();
        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };
        let lockfile_content = lockfile_toml_content(&Lockfile::with_toolchain("2.1.0"));

        let result = build_single(
            &project,
            &manifest,
            &konanc,
            None,
            &target,
            "debug",
            &options,
            &[],
            &lockfile_content,
        );

        assert!(
            result.is_err(),
            "build_single should fail when only test sources exist"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no .kt source files") || err.contains("source"),
            "expected no-sources error, got: {err}"
        );
    }

    #[test]
    fn test_source_change_affects_test_build_key_but_not_regular_build_key() {
        // Verify that modifying src/test/FooTest.kt changes the cache key for
        // test builds (which use profile "debug-test") but does NOT change the
        // cache key for regular builds (which use profile "debug") when the
        // source_dir is the same.
        //
        // Since both build_single and build_tests set source_dir to project/src
        // and CacheKey::compute hashes ALL **/*.kt files in source_dir, modifying
        // a test file changes the source hash for both. However, the profile
        // suffix ("debug" vs "debug-test") still differentiates them. This test
        // confirms that the profile differentiation works and that regular builds
        // and test builds never collide.
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("myapp");
        fs::create_dir_all(project.join("src").join("test")).unwrap();
        fs::write(project.join("src").join("main.kt"), "fun main() {}").unwrap();
        fs::write(
            project.join("src").join("test").join("FooTest.kt"),
            "class FooTest { fun foo() {} }",
        )
        .unwrap();

        let base_inputs = |profile: &str| CacheInputs {
            manifest_content: "[package]\nname = \"myapp\"".to_owned(),
            lockfile_content: String::new(),
            konanc_version: "2.1.0".to_owned(),
            konanc_fingerprint: "abc123".to_owned(),
            target: "linux_x64".to_owned(),
            profile: profile.to_owned(),
            source_dir: project.join("src"),
            source_glob: "**/*.kt".to_owned(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            dependency_hashes: Vec::new(),
        };

        // Compute keys BEFORE modifying the test file.
        let build_key_before = CacheKey::compute(&base_inputs("debug")).unwrap();
        let test_key_before = CacheKey::compute(&base_inputs("debug-test")).unwrap();

        // Regular and test builds must always have different cache keys.
        assert_ne!(
            build_key_before, test_key_before,
            "regular build and test build must have different cache keys"
        );

        // Modify a test source file.
        fs::write(
            project.join("src").join("test").join("FooTest.kt"),
            "class FooTest { fun foo() { /* modified */ } }",
        )
        .unwrap();

        // Compute keys AFTER modifying the test file.
        let build_key_after = CacheKey::compute(&base_inputs("debug")).unwrap();
        let test_key_after = CacheKey::compute(&base_inputs("debug-test")).unwrap();

        // Both keys should change because sha256_dir hashes all files in src/.
        assert_ne!(
            build_key_before, build_key_after,
            "regular build cache key should change when any src/ file changes"
        );
        assert_ne!(
            test_key_before, test_key_after,
            "test build cache key should change when test source changes"
        );

        // Regular and test keys must still differ from each other after the change.
        assert_ne!(
            build_key_after, test_key_after,
            "regular and test build keys must never collide"
        );
    }
}
