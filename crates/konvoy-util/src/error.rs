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
    #[error("artifact hash mismatch for {path}\n  expected: {expected}\n  got:      {actual}\n\n  This can happen if:\n    - the file was corrupted on disk (e.g. interrupted download, disk error)\n    - the file was tampered with (e.g. malware or unauthorized modification)\n    - the lockfile hashes are stale (e.g. manual edits to konvoy.lock)\n\n  To fix: inspect or delete the file above, then re-run the build.\n  If the problem persists, run `konvoy update` to re-resolve all dependencies.")]
    ArtifactHashMismatch {
        path: String,
        expected: String,
        actual: String,
    },

    /// Cannot determine the user's home directory.
    #[error("cannot determine home directory — set the HOME environment variable")]
    NoHomeDir,

    /// POM XML could not be parsed or is missing required elements.
    #[error("cannot parse POM: {reason}")]
    PomParse { reason: String },

    /// A POM dependency uses a version range instead of an exact version.
    #[error("version range \"{range}\" in POM dependency {group}:{artifact} is not supported — pin an exact version")]
    PomUnsupportedVersionRange {
        group: String,
        artifact: String,
        range: String,
    },

    /// A POM uses a property placeholder that konvoy does not resolve.
    #[error("unsupported property \"{property}\" in POM — only ${{project.version}} and ${{project.groupId}} are supported")]
    PomUnsupportedProperty { property: String },
}
