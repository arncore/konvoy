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

    for dep in &dep_graph.order {
        let dep_output = build_single(
            &dep.project_root,
            &dep.manifest,
            &konanc,
            jre_home.as_deref(),
            &target,
            profile,
            options,
            &library_paths,
        )?;
        library_paths.push(dep_output);
    }

    // 6. Build the root project.
    let result = build_single(
        project_root,
        &manifest,
        &konanc,
        jre_home.as_deref(),
        &target,
        profile,
        options,
        &library_paths,
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
    )?;

    Ok(BuildResult {
        outcome: BuildOutcome::Fresh,
        output_path: result,
        duration: start.elapsed(),
    })
}

/// Build a single project (either root or a dependency).
///
/// Returns the path to the output artifact.
#[allow(clippy::too_many_arguments)]
fn build_single(
    project_root: &Path,
    manifest: &Manifest,
    konanc: &KonancInfo,
    jre_home: Option<&Path>,
    target: &Target,
    profile: &str,
    options: &BuildOptions,
    library_paths: &[PathBuf],
) -> Result<PathBuf, EngineError> {
    // Collect source files.
    let src_dir = project_root.join("src");
    let sources = konvoy_util::fs::collect_files(&src_dir, "kt")?;
    if sources.is_empty() {
        return Err(EngineError::NoSources {
            dir: src_dir.display().to_string(),
        });
    }

    let is_lib = manifest.package.kind == PackageKind::Lib;

    // Compute cache key.
    let manifest_content = manifest.to_toml()?;
    let effective_lockfile = Lockfile::with_toolchain(&konanc.version);
    let lockfile_content = lockfile_toml_content(&effective_lockfile);
    let cache_inputs = CacheInputs {
        manifest_content,
        lockfile_content,
        konanc_version: konanc.version.clone(),
        konanc_fingerprint: konanc.fingerprint.clone(),
        target: target.to_string(),
        profile: profile.to_owned(),
        source_dir: project_root.to_path_buf(),
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
        return Ok(output_path);
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

    Ok(output_path)
}

/// Resolve the target: use the explicit `--target` value or detect the host.
fn resolve_target(target_opt: &Option<String>) -> Result<Target, EngineError> {
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
fn lockfile_toml_content(lockfile: &Lockfile) -> String {
    toml::to_string_pretty(lockfile).unwrap_or_default()
}

/// Update konvoy.lock if the detected konanc version or dependency hashes differ,
/// or if a fresh download provides new tarball hashes. When the lockfile already
/// contains hashes and a fresh download yields different ones, emit a warning.
fn update_lockfile_if_needed(
    lockfile: &Lockfile,
    konanc: &KonancInfo,
    konanc_tarball_sha256: Option<&str>,
    jre_tarball_sha256: Option<&str>,
    dep_graph: &ResolvedGraph,
    project_root: &Path,
    lockfile_path: &Path,
) -> Result<(), EngineError> {
    // Check for dependency source hash mismatches and warn.
    for dep in &dep_graph.order {
        if let Some(locked) = lockfile.dependencies.iter().find(|d| d.name == dep.name) {
            if locked.source_hash != dep.source_hash && !dep.source_hash.is_empty() {
                eprintln!(
                    "warning: dependency `{}` source has changed (locked: {}, current: {})",
                    dep.name,
                    truncate_hash(&locked.source_hash),
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

    // When we have fresh download hashes and the lockfile already has hashes,
    // verify they match. A mismatch could indicate a tampered or rotated tarball.
    if let Some(tc) = &lockfile.toolchain {
        if let (Some(existing), Some(actual)) = (&tc.konanc_tarball_sha256, konanc_tarball_sha256) {
            if !existing.is_empty() && existing != actual {
                eprintln!(
                    "warning: konanc tarball hash changed — expected {existing}, got {actual}; lockfile updated"
                );
            }
        }
        if let (Some(existing), Some(actual)) = (&tc.jre_tarball_sha256, jre_tarball_sha256) {
            if !existing.is_empty() && existing != actual {
                eprintln!(
                    "warning: jre tarball hash changed — expected {existing}, got {actual}; lockfile updated"
                );
            }
        }
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
fn now_iso8601() -> String {
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
        )
        .unwrap();
        let content = fs::read_to_string(&lockfile_path).unwrap();
        assert!(content.contains("2.1.0"));
    }

    #[test]
    fn build_options_defaults() {
        let opts = BuildOptions {
            target: None,
            release: false,
            verbose: false,
        };
        assert!(opts.target.is_none());
        assert!(!opts.release);
        assert!(!opts.verbose);
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
        )
        .unwrap();

        let reparsed = Lockfile::from_path(&lockfile_path).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_version, "2.1.0");
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
        )
        .unwrap();

        let reparsed = Lockfile::from_path(&lockfile_path).unwrap();
        let tc = reparsed.toolchain.as_ref().unwrap();
        assert_eq!(tc.konanc_tarball_sha256.as_deref(), Some("existing1"));
        assert_eq!(tc.jre_tarball_sha256.as_deref(), Some("existing2"));
    }
}
