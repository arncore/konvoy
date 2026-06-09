//! Declarative source generation before Kotlin/Native compilation.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use konvoy_config::lockfile::Lockfile;
use konvoy_config::manifest::Codegen;

use crate::error::EngineError;

mod managed_tool;
pub mod openapi;

pub use managed_tool::{ManagedToolResolution, ManagedToolSpec};

/// Display metadata for a configured generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratorSummary {
    /// Stable generator name used in paths and cache key tags.
    pub name: String,
    /// Human-readable generator label.
    pub display_name: String,
    /// Directory containing this generator's outputs.
    pub output_dir: PathBuf,
}

/// A configured code generator.
pub trait CodeGenerator {
    /// Stable generator name used in paths and cache key tags.
    fn name(&self) -> &str;

    /// Human-readable generator label.
    fn display_name(&self) -> &str;

    /// Managed tool required by this generator.
    fn managed_tool(&self) -> ManagedToolSpec;

    /// Stable config fields that affect generated sources.
    fn config_hash_parts(&self) -> Vec<String>;

    /// Project-relative input files read by this generator.
    ///
    /// `project_root` lets generators enumerate inputs that live on disk (e.g.
    /// every file under a configured spec directory). The returned paths are
    /// project-relative and their contents are folded into the generator hash
    /// (and thus the build cache key).
    ///
    /// # Errors
    /// Returns an error if a configured input location (e.g. a spec directory)
    /// is missing or cannot be read.
    fn input_files(&self, project_root: &Path) -> Result<Vec<PathBuf>, EngineError>;

    /// Generate sources into `output_dir`.
    ///
    /// # Errors
    /// Returns an error if the generator process cannot be executed or fails.
    fn generate(
        &self,
        project_root: &Path,
        output_dir: &Path,
        tool_path: &Path,
        jre_home: &Path,
        verbose: bool,
    ) -> Result<(), EngineError>;
}

/// Return all active generators in stable order.
pub fn active_generators(codegen: &Codegen) -> Vec<Box<dyn CodeGenerator>> {
    let mut generators: Vec<Box<dyn CodeGenerator>> = Vec::new();
    if let Some(openapi) = &codegen.openapi {
        generators.push(Box::new(openapi::OpenApiGenerator::new(openapi.clone())));
    }
    generators
}

/// Return display summaries for all active generators.
#[must_use]
pub fn generator_summaries(project_root: &Path, codegen: &Codegen) -> Vec<GeneratorSummary> {
    active_generators(codegen)
        .into_iter()
        .map(|generator| GeneratorSummary {
            name: generator.name().to_owned(),
            display_name: generator.display_name().to_owned(),
            output_dir: generator_output_dir(project_root, generator.name()),
        })
        .collect()
}

/// Return managed tools for all active generators.
#[must_use]
pub fn managed_tools(codegen: &Codegen) -> Vec<ManagedToolSpec> {
    active_generators(codegen)
        .into_iter()
        .map(|generator| generator.managed_tool())
        .collect()
}

/// Compute `(generator name, input hash)` pairs for all active generators, in
/// stable order.
///
/// This is the single place generator hashes are computed. A build computes them
/// once for the cache key and threads them into [`run_codegen`] (via its
/// `precomputed_hashes` argument) so neither the hash nor any spec / `spec_dirs`
/// file is read a second time on a cache-miss build.
///
/// # Errors
/// Returns an error if a configured generator input cannot be read.
pub fn compute_codegen_hash_pairs(
    project_root: &Path,
    codegen: &Codegen,
) -> Result<Vec<(String, String)>, EngineError> {
    active_generators(codegen)
        .into_iter()
        .map(|generator| {
            let hash = compute_generator_hash(generator.as_ref(), project_root)?;
            Ok((generator.name().to_owned(), hash))
        })
        .collect()
}

/// Compute tagged (`name:hash`) hashes for all active generators — the form
/// folded into the build cache key.
///
/// # Errors
/// Returns an error if a configured generator input cannot be read.
pub fn compute_codegen_hashes(
    project_root: &Path,
    codegen: &Codegen,
) -> Result<Vec<String>, EngineError> {
    Ok(compute_codegen_hash_pairs(project_root, codegen)?
        .into_iter()
        .map(|(name, hash)| format!("{name}:{hash}"))
        .collect())
}

/// Run configured generators when their inputs are stale, then collect generated `.kt` files.
///
/// `precomputed_hashes`, when provided, supplies each generator's input hash
/// (already computed once for the build cache key via
/// [`compute_codegen_hash_pairs`]) so it is not recomputed here — avoiding a
/// second read of every spec / `spec_dirs` file on a cache-miss build. Pass
/// `None` (e.g. from `konvoy generate`) to compute the hash on demand.
///
/// # Errors
/// Returns an error if tool resolution, source generation, or generated source
/// collection fails.
#[allow(clippy::too_many_arguments)]
pub fn run_codegen(
    project_root: &Path,
    codegen: &Codegen,
    lockfile: &Lockfile,
    lockfile_path: &Path,
    kotlin_version: &str,
    jre_home: Option<&Path>,
    verbose: bool,
    locked: bool,
    force: bool,
    precomputed_hashes: Option<&[(String, String)]>,
) -> Result<Vec<PathBuf>, EngineError> {
    let generators = active_generators(codegen);
    if generators.is_empty() {
        return Ok(Vec::new());
    }

    let mut generated_sources = Vec::new();

    for generator in generators {
        // Reuse the hash computed for the cache key when the caller threaded it
        // in; otherwise compute it now. Match by name so order is irrelevant.
        let input_hash = match precomputed_hashes.and_then(|pairs| {
            pairs
                .iter()
                .find(|(name, _)| name.as_str() == generator.name())
        }) {
            Some((_, hash)) => hash.clone(),
            None => compute_generator_hash(generator.as_ref(), project_root)?,
        };
        let output_dir = generator_output_dir(project_root, generator.name());
        let input_hash_path = output_dir.join(".input_hash");
        let stored_hash = std::fs::read_to_string(&input_hash_path).ok();
        let stale = force
            || stored_hash
                .as_deref()
                .is_none_or(|stored| stored.trim() != input_hash);

        let tool_spec = generator.managed_tool();
        let needs_tool = stale || locked || needs_tool_lock_update(lockfile, &tool_spec);
        let tool = if needs_tool {
            Some(resolve_managed_tool(&tool_spec, lockfile, locked)?)
        } else {
            None
        };

        if let Some(tool_resolution) = &tool {
            if tool_resolution.should_persist {
                persist_managed_tool_hash(
                    lockfile_path,
                    lockfile,
                    &tool_spec,
                    &tool_resolution.sha256,
                )?;
            }
        }

        if stale {
            let resolved_jre_home = resolve_jre_home(kotlin_version, jre_home)?;
            let Some(tool_resolution) = tool.as_ref() else {
                return Err(EngineError::InternalInvariantViolated {
                    context: "stale codegen input without resolved tool".to_owned(),
                });
            };

            konvoy_util::fs::remove_dir_all_if_exists(&output_dir)?;
            konvoy_util::fs::ensure_dir(&output_dir)?;
            generator.generate(
                project_root,
                &output_dir,
                &tool_resolution.path,
                &resolved_jre_home,
                verbose,
            )?;
            konvoy_util::fs::write_file(&input_hash_path, format!("{input_hash}\n"))?;
        }

        if output_dir.exists() {
            let mut sources = konvoy_util::fs::collect_files(&output_dir, "kt")?;
            generated_sources.append(&mut sources);
        }
    }

    generated_sources.sort();
    Ok(generated_sources)
}

/// Return the output directory for a generator under `.konvoy/gen/`.
#[must_use]
pub fn generator_output_dir(project_root: &Path, name: &str) -> PathBuf {
    project_root.join(".konvoy").join("gen").join(name)
}

fn resolve_jre_home(
    kotlin_version: &str,
    existing_jre_home: Option<&Path>,
) -> Result<PathBuf, EngineError> {
    if let Some(jre_home) = existing_jre_home {
        return Ok(jre_home.to_path_buf());
    }

    // Codegen only needs a JRE to run the tool JAR, so install just the managed
    // JRE instead of downloading the full Kotlin/Native compiler toolchain.
    Ok(konvoy_konanc::toolchain::ensure_jre(kotlin_version)?)
}

fn compute_generator_hash(
    generator: &dyn CodeGenerator,
    project_root: &Path,
) -> Result<String, EngineError> {
    let mut parts = vec![
        "codegen-v1".to_owned(),
        generator.name().to_owned(),
        generator.display_name().to_owned(),
    ];
    parts.extend(generator.config_hash_parts());

    for input in generator.input_files(project_root)? {
        let full_path = project_root.join(&input);
        if !full_path.exists() {
            return Err(EngineError::CodegenInputNotFound {
                name: generator.name().to_owned(),
                path: full_path.display().to_string(),
            });
        }
        parts.push(format!("file:{}", input.display()));
        parts.push(konvoy_util::hash::sha256_file(&full_path)?);
    }

    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    Ok(konvoy_util::hash::sha256_multi(&refs))
}

fn resolve_managed_tool(
    spec: &ManagedToolSpec,
    lockfile: &Lockfile,
    locked: bool,
) -> Result<ManagedToolResolution, EngineError> {
    let pin = lockfile.codegen_tool(&spec.id);
    let matching_pin = pin
        .as_ref()
        .filter(|candidate| candidate.version == spec.version());

    if locked {
        if matching_pin.is_none() {
            return Err(EngineError::LockfileUpdateRequired);
        }
        if !spec.is_installed()? {
            return Err(EngineError::CodegenToolNotFound {
                name: spec.id.clone(),
                version: spec.version().to_owned(),
            });
        }
    }

    let expected_hash = matching_pin.map(|candidate| candidate.sha256.as_str());
    let (path, sha256) = spec.ensure(expected_hash)?;
    let generic_pin_is_current = lockfile
        .codegen_tools
        .get(&spec.id)
        .is_some_and(|generic_pin| {
            generic_pin.version == spec.version() && !generic_pin.sha256.trim().is_empty()
        });
    Ok(ManagedToolResolution {
        path,
        sha256,
        should_persist: !generic_pin_is_current,
    })
}

fn needs_tool_lock_update(lockfile: &Lockfile, spec: &ManagedToolSpec) -> bool {
    !lockfile.has_codegen_tool(&spec.id, spec.version())
}

/// Serializes the read-modify-write of `konvoy.lock` across the codegen tool
/// persistence path. Path-dependency builds run `build_single` (and therefore
/// `run_codegen`) in parallel, and each generator may persist its tool pin into
/// the shared root lockfile; without this guard two threads could lose each
/// other's update or interleave a write.
static CODEGEN_LOCK_PERSIST: Mutex<()> = Mutex::new(());

fn persist_managed_tool_hash(
    lockfile_path: &Path,
    lockfile: &Lockfile,
    spec: &ManagedToolSpec,
    hash: &str,
) -> Result<(), EngineError> {
    // Hold the lock for the entire read-modify-write so concurrent persists are
    // atomic with respect to one another. A poisoned lock only means another
    // persist panicked; the data itself is still consistent, so recover it.
    let _guard = CODEGEN_LOCK_PERSIST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut updated = if lockfile_path.exists() {
        Lockfile::from_path(lockfile_path)?
    } else {
        lockfile.clone()
    };
    // Persist only the codegen tool pin. The toolchain section (with its konanc/
    // JRE tarball hashes) is owned by update_lockfile_if_needed, which runs after
    // a successful build. Writing a stub toolchain here would leave null tarball
    // hashes behind if the build then fails before that point.
    updated.set_codegen_tool(&spec.id, spec.version(), hash);
    updated.write_to(lockfile_path)?;
    Ok(())
}

/// Run a managed Java JAR with normalized diagnostics.
///
/// # Errors
/// Returns an error if Java is unavailable, the process cannot be spawned, or
/// the process exits unsuccessfully.
pub fn run_java_jar(
    generator_name: &str,
    tool_display_name: &str,
    tool_path: &Path,
    jre_home: &Path,
    args: Vec<OsString>,
    verbose: bool,
) -> Result<(), EngineError> {
    let java = java_bin(jre_home);
    if !java.exists() {
        return Err(EngineError::CodegenFailed {
            name: generator_name.to_owned(),
            message: format!(
                "java not found at {} — run `konvoy toolchain install` to reinstall the managed JRE",
                java.display()
            ),
        });
    }

    let output = Command::new(&java)
        .arg("-jar")
        .arg(tool_path)
        .args(args)
        .env("JAVA_HOME", jre_home)
        .output()
        .map_err(|e| EngineError::CodegenFailed {
            name: generator_name.to_owned(),
            message: e.to_string(),
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if verbose {
        if !stdout.is_empty() {
            eprintln!("{stdout}");
        }
        if !stderr.is_empty() {
            eprintln!("{stderr}");
        }
    }

    if !output.status.success() {
        // JVM CLIs typically print the real error to stderr (and banners to
        // stdout), so prefer stderr for the one-line hint and fall back to
        // stdout. Never concatenate the streams — that can fuse an unrelated
        // stdout line onto the first stderr line.
        let hint_source = if stderr.trim().is_empty() {
            stdout.as_ref()
        } else {
            stderr.as_ref()
        };
        let hint = first_non_empty_line(hint_source)
            .map(|line| format!(" first message: {line}"))
            .unwrap_or_default();
        return Err(EngineError::CodegenFailed {
            name: generator_name.to_owned(),
            message: format!(
                "{tool_display_name} exited with status {}.{hint} Run with --verbose to see full output.",
                output.status
            ),
        });
    }

    Ok(())
}

fn java_bin(jre_home: &Path) -> PathBuf {
    let binary = if cfg!(windows) { "java.exe" } else { "java" };
    jre_home.join("bin").join(binary)
}

fn first_non_empty_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}
