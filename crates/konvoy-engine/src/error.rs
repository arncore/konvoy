//! Error types for konvoy-engine.

/// Errors produced by engine operations.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A filesystem operation failed.
    #[error("cannot access {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    /// A utility operation failed.
    #[error("{0}")]
    Util(#[from] konvoy_util::error::UtilError),

    /// A manifest operation failed.
    #[error("{0}")]
    Manifest(#[from] konvoy_config::manifest::ManifestError),

    /// A project already exists at the target path.
    #[error("konvoy.toml already exists at {path} — cannot initialize over an existing project")]
    ProjectExists { path: String },

    /// Metadata serialization/deserialization failed.
    #[error("cannot process metadata: {message}")]
    Metadata { message: String },

    /// A compiler operation failed.
    #[error("{0}")]
    Konanc(konvoy_konanc::error::KonancError),

    /// A target resolution failed.
    #[error("{0}")]
    Target(konvoy_targets::TargetError),

    /// Lockfile error.
    #[error("lockfile error: {0}")]
    Lockfile(String),

    /// No source files found.
    #[error("no .kt source files found in {dir}")]
    NoSources { dir: String },

    /// Compilation failed.
    #[error("compilation failed with {error_count} error(s)")]
    CompilationFailed { error_count: usize },
}
