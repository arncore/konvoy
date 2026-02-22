//! Error types for konvoy-util.

/// Errors produced by utility functions.
#[derive(Debug, thiserror::Error)]
pub enum UtilError {
    /// An I/O operation failed.
    #[error("cannot access {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },

    /// A glob pattern was invalid.
    #[error("invalid glob pattern `{pattern}`: {message}")]
    GlobPattern { pattern: String, message: String },

    /// A command failed to execute.
    #[error("cannot execute command: {source}")]
    CommandExec { source: std::io::Error },
}
