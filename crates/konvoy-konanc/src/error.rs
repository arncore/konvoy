//! Error types for konvoy-konanc.

use std::path::PathBuf;

/// Errors produced by compiler detection and invocation.
#[derive(Debug, thiserror::Error)]
pub enum KonancError {
    /// konanc binary was not found on the system.
    #[error("konanc not found — install Kotlin/Native and add it to PATH, or set KONANC_HOME")]
    NotFound,

    /// konanc was found but is not executable.
    #[error("konanc found at {path} but is not executable — check file permissions")]
    NotExecutable { path: PathBuf },

    /// Failed to execute konanc.
    #[error("cannot execute konanc: {source}")]
    Exec { source: std::io::Error },

    /// konanc -version returned an unexpected format.
    #[error("cannot parse konanc version from output: {output}")]
    VersionParse { output: String },

    /// Cannot compute fingerprint of the konanc binary.
    #[error("cannot fingerprint konanc binary at {path}: {source}")]
    Fingerprint {
        path: PathBuf,
        source: konvoy_util::error::UtilError,
    },

    /// Compilation failed.
    #[error("compilation failed with {error_count} error(s)")]
    CompilationFailed { error_count: usize },

    /// No source files provided.
    #[error("no source files specified — add .kt files to the sources list")]
    NoSources,

    /// No output path specified.
    #[error("no output path specified — set the output binary path")]
    NoOutput,

    /// Platform toolchain is missing.
    #[error("{message} — run `{fix_command}`")]
    MissingToolchain {
        message: String,
        fix_command: String,
    },
}
