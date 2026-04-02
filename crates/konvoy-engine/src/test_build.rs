//! Test build orchestration: compile and run Kotlin/Native tests using konanc's
//! built-in test runner (`-generate-test-runner` flag with `kotlin.test`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use crate::artifact::{ArtifactStore, BuildMetadata};
use crate::build::{
    lockfile_toml_content, now_epoch_secs, resolve_target, BuildOptions, BuildOutcome,
};
use crate::cache::{CacheInputs, CacheKey};
use crate::error::EngineError;
use crate::resolve::resolve_dependencies;
use konvoy_config::lockfile::Lockfile;
use konvoy_config::manifest::Manifest;
use konvoy_konanc::detect::resolve_konanc;
use konvoy_konanc::invoke::{KonancCommand, ProduceKind};

/// Result of a successful test build.
#[derive(Debug)]
pub struct TestBuildResult {
    /// Whether the build used cache or compiled fresh.
    pub outcome: BuildOutcome,
    /// Path to the test binary.
    pub output_path: PathBuf,
    /// How long the build took.
    pub compile_duration: std::time::Duration,
}

/// Build test sources using konanc's built-in test runner.
///
/// Collects both project sources (`src/**/*.kt` excluding `src/test/`) and test
/// sources (`src/test/**/*.kt`), then invokes konanc with `-generate-test-runner`
/// to produce a test binary.
///
/// The build pipeline mirrors `build()`: auto-resolve Maven deps, staleness
/// checks, lockfile pre-stabilization, parallel dependency builds, plugin and
/// Maven klib resolution.
///
/// # Errors
/// Returns an error if test sources are missing, compilation fails, or any
/// filesystem operation fails.
pub fn build_tests(
    project_root: &Path,
    options: &BuildOptions,
) -> Result<TestBuildResult, EngineError> {
    let start = Instant::now();

    // 1. Read konvoy.toml.
    let manifest_path = project_root.join("konvoy.toml");
    let manifest = Manifest::from_path(&manifest_path)?;

    // 2. Read konvoy.lock (or default).
    let lockfile_path = project_root.join("konvoy.lock");
    let lockfile = Lockfile::from_path(&lockfile_path)?;

    // 3. Auto-resolve Maven deps if needed (unless --locked).
    let lockfile =
        if !options.locked && crate::build::has_unresolved_maven_deps(&manifest, &lockfile) {
            eprintln!("  Maven dependencies not resolved \u{2014} running update automatically...");
            crate::update::update(project_root)?;
            Lockfile::from_path(&lockfile_path)?
        } else {
            lockfile
        };

    // In --locked mode, verify the lockfile is complete and consistent.
    if options.locked {
        crate::build::check_lockfile_staleness(&manifest, &lockfile)?;
    }

    // 4. Resolve target.
    let target = resolve_target(&options.target)?;
    let profile = if options.release { "release" } else { "debug" };

    // 5. Resolve managed konanc toolchain.
    let resolved = resolve_konanc(&manifest.toolchain.kotlin)?;
    let jre_home = resolved.jre_home.clone();
    let konanc = resolved.info;

    // 6. Pre-stabilize the lockfile for cache key consistency.
    let effective_lockfile = if options.locked {
        lockfile
    } else {
        match &lockfile.toolchain {
            Some(tc) if tc.konanc_version == konanc.version => lockfile.clone(),
            _ => {
                let mut stabilized = Lockfile::with_managed_toolchain(
                    &konanc.version,
                    resolved.konanc_tarball_sha256.as_deref(),
                    resolved.jre_tarball_sha256.as_deref(),
                );
                stabilized.dependencies = lockfile.dependencies.clone();
                stabilized.plugins = lockfile.plugins.clone();
                stabilized
            }
        }
    };

    // 7. Resolve dependencies and build them in parallel (topological levels).
    let dep_graph = resolve_dependencies(project_root, &manifest)?;
    let lockfile_content = lockfile_toml_content(&effective_lockfile)?;
    let levels = crate::resolve::parallel_levels(&dep_graph);
    let mut completed: HashMap<String, PathBuf> = HashMap::new();

    for level in &levels {
        let lib_paths: Vec<PathBuf> = completed.values().cloned().collect();

        let results: Vec<Result<(String, PathBuf, BuildOutcome), EngineError>> = level
            .par_iter()
            .map(|dep| {
                let (output, outcome) = crate::build::build_single(
                    &dep.project_root,
                    &dep.manifest,
                    &konanc,
                    jre_home.as_deref(),
                    &target,
                    profile,
                    options,
                    &lib_paths,
                    &[],
                    &lockfile_content,
                )?;
                Ok((dep.name.clone(), output, outcome))
            })
            .collect();

        for result in results {
            let (name, output, _) = result?;
            completed.insert(name, output);
        }
    }

    let mut library_paths: Vec<PathBuf> = dep_graph
        .order
        .iter()
        .filter_map(|dep| completed.get(&dep.name).cloned())
        .collect();

    // 7a. Resolve and download plugin artifacts.
    let plugin_jars = if !manifest.plugins.is_empty() {
        let resolved_artifacts = crate::plugin::resolve_plugin_artifacts(&manifest)?;
        let results = crate::plugin::ensure_plugin_artifacts(
            &resolved_artifacts,
            &effective_lockfile,
            options.locked,
        )?;
        results.iter().map(|r| r.path.clone()).collect()
    } else {
        Vec::new()
    };

    // 7b. Resolve and download Maven dependency klibs for the current target.
    let maven_klibs = crate::build::resolve_maven_deps(&effective_lockfile, &target)?;
    library_paths.extend(maven_klibs);

    // Collect project sources (excluding src/test/) and test sources.
    let src_dir = project_root.join("src");
    let test_dir = src_dir.join("test");

    if !test_dir.is_dir() {
        return Err(EngineError::NoTestSources {
            dir: test_dir.display().to_string(),
        });
    }

    let test_sources = konvoy_util::fs::collect_files(&test_dir, "kt")?;
    if test_sources.is_empty() {
        return Err(EngineError::NoTestSources {
            dir: test_dir.display().to_string(),
        });
    }

    // Collect main sources but exclude the test directory.
    let all_sources = konvoy_util::fs::collect_files(&src_dir, "kt")?;
    let mut sources: Vec<PathBuf> = all_sources
        .into_iter()
        .filter(|p| !p.starts_with(&test_dir))
        .collect();
    sources.extend(test_sources);

    // Compute cache key (includes test sources via source hashing).
    let manifest_content = manifest.to_toml()?;
    let cache_inputs = CacheInputs {
        manifest_content,
        lockfile_content,
        konanc_version: konanc.version.clone(),
        konanc_fingerprint: konanc.fingerprint.clone(),
        target: target.to_string(),
        // Use "debug-test" / "release-test" to differentiate from regular builds.
        profile: format!("{profile}-test"),
        source_dir: project_root.join("src"),
        source_glob: "**/*.kt".to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        dependency_hashes: library_paths
            .iter()
            .map(|p| konvoy_util::hash::sha256_file(p).map_err(EngineError::from))
            .collect::<Result<Vec<_>, _>>()?,
    };
    let cache_key = CacheKey::compute(&cache_inputs)?;

    let output_name = format!("test-{}", manifest.package.name);
    let output_path = project_root
        .join(".konvoy")
        .join("build")
        .join(target.to_konanc_arg())
        .join(profile)
        .join(&output_name);

    let store = ArtifactStore::new(project_root);

    // Check cache (respecting --force).
    if !options.force && store.has(&cache_key) {
        eprintln!("    Fresh {} (cached)", output_name);
        store.materialize(&cache_key, &output_name, &output_path)?;
        return Ok(TestBuildResult {
            outcome: BuildOutcome::Cached,
            output_path,
            compile_duration: start.elapsed(),
        });
    }

    // Compile with test runner generation.
    eprintln!(
        "    Compiling {} \u{2192} {}",
        output_name,
        output_path.display()
    );

    if let Some(parent) = output_path.parent() {
        konvoy_util::fs::ensure_dir(parent)?;
    }

    let mut cmd = KonancCommand::new()
        .sources(&sources)
        .output(&output_path)
        .target(target.to_konanc_arg())
        .release(options.release)
        .produce(ProduceKind::Program)
        .generate_test_runner(true)
        .libraries(&library_paths)
        .plugins(&plugin_jars);

    if let Some(jh) = jre_home.as_deref() {
        cmd = cmd.java_home(jh);
    }

    let result = cmd.execute(&konanc)?;

    crate::diagnostics::print_diagnostics(&result, options.verbose);

    if !result.success {
        return Err(EngineError::CompilationFailed {
            error_count: result.error_count(),
        });
    }

    // Handle .kexe suffix on Linux.
    crate::build::normalize_konanc_output(&output_path)?;

    // Store in cache.
    let metadata = BuildMetadata {
        target: target.to_string(),
        profile: profile.to_owned(),
        konanc_version: konanc.version,
        built_at: now_epoch_secs(),
    };
    store.store(&cache_key, &output_path, &metadata)?;

    Ok(TestBuildResult {
        outcome: BuildOutcome::Fresh,
        output_path,
        compile_duration: start.elapsed(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn build_tests_fails_without_test_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("myapp");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src").join("main.kt"), "fun main() {}").unwrap();
        fs::write(
            project.join("konvoy.toml"),
            "[package]\nname = \"myapp\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
        )
        .unwrap();

        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };

        let result = build_tests(&project, &options);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no test source files"),
            "expected no test sources error, got: {err}"
        );
    }

    #[test]
    fn build_tests_fails_with_empty_test_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("myapp");
        fs::create_dir_all(project.join("src").join("test")).unwrap();
        fs::write(project.join("src").join("main.kt"), "fun main() {}").unwrap();
        fs::write(
            project.join("konvoy.toml"),
            "[package]\nname = \"myapp\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
        )
        .unwrap();

        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };

        let result = build_tests(&project, &options);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no test source files"),
            "expected no test sources error, got: {err}"
        );
    }

    #[test]
    fn build_tests_fails_without_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let options = BuildOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };

        let result = build_tests(tmp.path(), &options);
        assert!(result.is_err());
    }
}
