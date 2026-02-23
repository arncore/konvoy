//! Test build orchestration: compile and run Kotlin/Native tests using konanc's
//! built-in test runner (`-generate-test-runner` flag with `kotlin.test`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::artifact::{ArtifactStore, BuildMetadata};
use crate::build::{lockfile_toml_content, now_iso8601, resolve_target, BuildOutcome};
use crate::cache::{CacheInputs, CacheKey};
use crate::error::EngineError;
use crate::resolve::resolve_dependencies;
use konvoy_config::lockfile::Lockfile;
use konvoy_config::manifest::Manifest;
use konvoy_konanc::detect::resolve_konanc;
use konvoy_konanc::invoke::{DiagnosticLevel, KonancCommand, ProduceKind};

/// Options controlling a test build invocation.
#[derive(Debug, Clone)]
pub struct TestOptions {
    /// Explicit target triple, or `None` for host.
    pub target: Option<String>,
    /// Whether to build tests in release mode.
    pub release: bool,
    /// Whether to show raw compiler output.
    pub verbose: bool,
    /// Allow overriding hash mismatch checks.
    pub force: bool,
    /// Require the lockfile to be up-to-date; error on any mismatch.
    pub locked: bool,
}

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
/// # Errors
/// Returns an error if test sources are missing, compilation fails, or any
/// filesystem operation fails.
pub fn build_tests(
    project_root: &Path,
    options: &TestOptions,
) -> Result<TestBuildResult, EngineError> {
    let start = Instant::now();

    let manifest_path = project_root.join("konvoy.toml");
    let manifest = Manifest::from_path(&manifest_path)?;

    let lockfile_path = project_root.join("konvoy.lock");
    let lockfile =
        Lockfile::from_path(&lockfile_path).map_err(|e| EngineError::Lockfile(e.to_string()))?;

    let target = resolve_target(&options.target)?;
    let profile = if options.release { "release" } else { "debug" };

    let resolved = resolve_konanc(&manifest.toolchain.kotlin).map_err(EngineError::Konanc)?;
    let jre_home = resolved.jre_home;
    let konanc = resolved.info;

    // Build dependencies first (same as regular build).
    let dep_graph = resolve_dependencies(project_root, &manifest)?;
    let mut library_paths: Vec<PathBuf> = Vec::new();
    let lockfile_content = lockfile_toml_content(&lockfile);

    for dep in &dep_graph.order {
        let (dep_output, _) = crate::build::build_single(
            &dep.project_root,
            &dep.manifest,
            &konanc,
            jre_home.as_deref(),
            &target,
            profile,
            &crate::build::BuildOptions {
                target: options.target.clone(),
                release: options.release,
                verbose: options.verbose,
                force: options.force,
                locked: options.locked,
            },
            &library_paths,
            &lockfile_content,
        )?;
        library_paths.push(dep_output);
    }

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
            .filter_map(|p| konvoy_util::hash::sha256_file(p).ok())
            .collect(),
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

    // Check cache.
    if store.has(&cache_key) {
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
        .libraries(&library_paths);

    if let Some(jh) = jre_home.as_deref() {
        cmd = cmd.java_home(jh);
    }

    let result = cmd.execute(&konanc).map_err(EngineError::Konanc)?;

    // Print diagnostics.
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

    // Handle .kexe suffix on Linux.
    let kexe_path = output_path.with_extension("kexe");
    if !output_path.exists() && kexe_path.exists() {
        std::fs::rename(&kexe_path, &output_path).map_err(|source| EngineError::Io {
            path: output_path.display().to_string(),
            source,
        })?;
    }

    // Store in cache.
    let metadata = BuildMetadata {
        target: target.to_string(),
        profile: profile.to_owned(),
        konanc_version: konanc.version,
        built_at: now_iso8601(),
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

        let options = TestOptions {
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

        let options = TestOptions {
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
        let options = TestOptions {
            target: None,
            release: false,
            verbose: false,
            force: false,
            locked: false,
        };

        let result = build_tests(tmp.path(), &options);
        assert!(result.is_err());
    }

    #[test]
    fn test_options_defaults() {
        let opts = TestOptions {
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
}
