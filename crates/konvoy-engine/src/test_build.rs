//! Test build orchestration: compile and run Kotlin/Native tests using konanc's
//! built-in test runner (`-generate-test-runner` flag with `kotlin.test`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::artifact::BuildMetadata;
use crate::build::{now_epoch_secs, resolve_build_context, BuildOptions, BuildOutcome};
use crate::cache::{CacheInputs, CacheKey};
use crate::error::EngineError;
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
/// Uses `resolve_build_context()` for the shared pipeline (steps 1–7b), then
/// adds test-specific source collection and compilation.
///
/// # Errors
/// Returns an error if test sources are missing, compilation fails, or any
/// filesystem operation fails.
pub fn build_tests(
    project_root: &Path,
    options: &BuildOptions,
) -> Result<TestBuildResult, EngineError> {
    let start = Instant::now();
    let ctx = resolve_build_context(project_root, options)?;

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
    let manifest_content = ctx.manifest.to_toml()?;
    let cache_inputs = CacheInputs {
        manifest_content,
        lockfile_content: ctx.lockfile_content,
        konanc_version: ctx.konanc.version.clone(),
        konanc_fingerprint: ctx.konanc.fingerprint.clone(),
        target: ctx.target.to_string(),
        // Use "debug-test" / "release-test" to differentiate from regular builds.
        profile: format!("{}-test", ctx.profile),
        source_dir: project_root.join("src"),
        source_glob: "**/*.kt".to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        dependency_hashes: ctx
            .library_paths
            .iter()
            .map(|p| konvoy_util::hash::sha256_file(p).map_err(EngineError::from))
            .collect::<Result<Vec<_>, _>>()?,
    };
    let cache_key = CacheKey::compute(&cache_inputs)?;

    let output_name = format!("test-{}", ctx.manifest.package.name);
    let output_path = project_root
        .join(".konvoy")
        .join("build")
        .join(ctx.target.to_konanc_arg())
        .join(&ctx.profile)
        .join(&output_name);

    // Check cache (respecting --force).
    if !options.force && ctx.store.has(&cache_key) {
        eprintln!("    Fresh {} (cached)", output_name);
        ctx.store
            .materialize(&cache_key, &output_name, &output_path)?;
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
        .target(ctx.target.to_konanc_arg())
        .release(options.release)
        .produce(ProduceKind::Program)
        .generate_test_runner(true)
        .libraries(&ctx.library_paths)
        .plugins(&ctx.plugin_jars);

    if let Some(jh) = ctx.jre_home.as_deref() {
        cmd = cmd.java_home(jh);
    }

    let result = cmd.execute(&ctx.konanc)?;

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
        target: ctx.target.to_string(),
        profile: ctx.profile,
        konanc_version: ctx.konanc.version,
        built_at: now_epoch_secs(),
    };
    ctx.store.store(&cache_key, &output_path, &metadata)?;

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
