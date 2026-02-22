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
}
