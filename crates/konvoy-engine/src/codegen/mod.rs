//! Declarative source generation before Kotlin/Native compilation.

use std::path::{Path, PathBuf};

use konvoy_config::manifest::Codegen;

use crate::error::EngineError;

pub mod openapi;

// The managed-tool abstraction is shared with the detekt linter, so it lives at
// the engine root (`crate::managed_tool`); re-exported here for codegen callers.
pub use crate::managed_tool::ManagedToolSpec;

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
    /// The tool returned by [`managed_tool`](Self::managed_tool) must already be
    /// downloaded; the generator runs it through
    /// [`ManagedToolSpec::run`](crate::managed_tool::ManagedToolSpec::run). `jre_home`
    /// is the managed JRE for JVM generators (e.g. Fabrikt) and `None` for a future
    /// native generator — it is forwarded to `run` per the tool's runtime.
    ///
    /// # Errors
    /// Returns an error if the generator process cannot be executed or fails.
    fn generate(
        &self,
        project_root: &Path,
        output_dir: &Path,
        jre_home: Option<&Path>,
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
/// once for the cache key and threads them into `run_codegen` (via its
/// `precomputed_hashes` argument) so neither the hash nor any spec /
/// `extra_spec_dirs` file is read a second time on a cache-miss build.
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

/// Return the output directory for a generator under `.konvoy/gen/`.
#[must_use]
pub fn generator_output_dir(project_root: &Path, name: &str) -> PathBuf {
    project_root.join(".konvoy").join("gen").join(name)
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
        // Hash the path's raw bytes (not Display, which is lossy for non-UTF-8
        // names) so a rename always changes the key and the key is encoding-stable.
        parts.push("file".to_owned());
        parts.push(konvoy_util::hash::sha256_bytes(
            input.as_os_str().as_encoded_bytes(),
        ));
        // Read content directly (no exists() pre-check): that races with the read
        // and reports EACCES as "not found". Map only a genuine NotFound to the
        // actionable codegen error; surface other I/O errors (e.g. permission) as-is.
        match konvoy_util::hash::sha256_file(&full_path) {
            Ok(hash) => parts.push(hash),
            Err(konvoy_util::error::UtilError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(EngineError::CodegenInputNotFound {
                    name: generator.name().to_owned(),
                    path: full_path.display().to_string(),
                });
            }
            Err(e) => return Err(EngineError::from(e)),
        }
    }

    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    Ok(konvoy_util::hash::sha256_multi(&refs))
}
