//! Declarative source generation before Kotlin/Native compilation.

use std::path::{Path, PathBuf};

use konvoy_config::lockfile::Lockfile;
use konvoy_config::manifest::Codegen;

use crate::error::EngineError;

pub mod openapi;

/// Result of resolving a managed codegen tool.
#[derive(Debug, Clone)]
pub struct ToolResolution {
    /// Path to the executable artifact, usually a JAR.
    pub path: PathBuf,
    /// SHA-256 hash of the tool artifact.
    pub sha256: String,
    /// Whether the hash should be persisted into `konvoy.lock`.
    pub should_persist: bool,
}

/// A configured code generator.
pub trait CodeGenerator {
    /// Stable generator name used in paths and cache key tags.
    fn name(&self) -> &str;

    /// Compute the hash of generator inputs, such as an OpenAPI spec file.
    ///
    /// # Errors
    /// Returns an error if any configured input cannot be read.
    fn compute_input_hash(&self, project_root: &Path) -> Result<String, EngineError>;

    /// Ensure the generator tool is available and hash-verified.
    ///
    /// # Errors
    /// Returns an error if the tool is missing in locked mode, cannot be
    /// downloaded, or does not match the lockfile hash.
    fn ensure_tool(&self, lockfile: &Lockfile, locked: bool)
        -> Result<ToolResolution, EngineError>;

    /// Persist the verified tool hash to `konvoy.lock`.
    ///
    /// # Errors
    /// Returns an error if the lockfile cannot be written.
    fn persist_tool_hash(
        &self,
        lockfile_path: &Path,
        lockfile: &Lockfile,
        kotlin_version: &str,
        hash: &str,
    ) -> Result<(), EngineError>;

    /// Return `true` if this generator needs a tool hash written to the lockfile.
    fn needs_tool_lock_update(&self, lockfile: &Lockfile) -> bool;

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

/// Compute tagged input hashes for all active generators.
///
/// # Errors
/// Returns an error if a configured generator input cannot be read.
pub fn compute_codegen_hashes(
    project_root: &Path,
    codegen: &Codegen,
) -> Result<Vec<String>, EngineError> {
    active_generators(codegen)
        .into_iter()
        .map(|generator| {
            let hash = generator.compute_input_hash(project_root)?;
            Ok(format!("{}:{hash}", generator.name()))
        })
        .collect()
}

/// Run configured generators when their inputs are stale, then collect generated `.kt` files.
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
) -> Result<Vec<PathBuf>, EngineError> {
    let generators = active_generators(codegen);
    if generators.is_empty() {
        return Ok(Vec::new());
    }

    let mut generated_sources = Vec::new();

    for generator in generators {
        let input_hash = generator.compute_input_hash(project_root)?;
        let output_dir = generator_output_dir(project_root, generator.name());
        let input_hash_path = output_dir.join(".input_hash");
        let stored_hash = std::fs::read_to_string(&input_hash_path).ok();
        let stale = stored_hash
            .as_deref()
            .is_none_or(|stored| stored.trim() != input_hash);

        let needs_tool = stale || locked || generator.needs_tool_lock_update(lockfile);
        let tool = if needs_tool {
            Some(generator.ensure_tool(lockfile, locked)?)
        } else {
            None
        };

        if let Some(tool_resolution) = &tool {
            if tool_resolution.should_persist {
                generator.persist_tool_hash(
                    lockfile_path,
                    lockfile,
                    kotlin_version,
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

    if !konvoy_konanc::toolchain::is_installed(kotlin_version)? {
        eprintln!("    Installing Kotlin/Native {kotlin_version} (for codegen JRE)...");
        konvoy_konanc::toolchain::install(kotlin_version)?;
    }

    Ok(konvoy_konanc::toolchain::jre_home_path(kotlin_version)?)
}
