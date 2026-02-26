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

    /// A download failed.
    #[error("download failed: {message}")]
    Download { message: String },

    /// A Maven coordinate string is malformed.
    #[error("invalid Maven coordinate \"{coordinate}\": {reason}")]
    InvalidMavenCoordinate { coordinate: String, reason: String },

    /// A version string contains unsafe characters.
    #[error("invalid version \"{version}\": only alphanumeric characters, dots, hyphens, and underscores are allowed")]
    InvalidVersion { version: String },

    /// An artifact hash does not match the expected value.
    #[error("artifact hash mismatch for {path} — expected {expected}, got {actual}")]
    ArtifactHashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    /// Cannot determine the user's home directory.
    #[error("cannot determine home directory — set the HOME environment variable")]
    NoHomeDir,
}
