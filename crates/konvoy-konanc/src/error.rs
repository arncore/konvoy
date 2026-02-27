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

    /// An error propagated from konvoy-util.
    #[error("{0}")]
    Util(#[from] konvoy_util::error::UtilError),

    /// The host platform is not supported for managed toolchain downloads.
    #[error("unsupported platform {os}/{arch} — Kotlin/Native prebuilt binaries are not available for this platform")]
    UnsupportedPlatform { os: String, arch: String },

    /// Failed to download a toolchain tarball.
    #[error("cannot download Kotlin/Native {version}: {message}")]
    Download { version: String, message: String },

    /// Failed to extract a toolchain tarball.
    #[error("cannot extract Kotlin/Native {version}: {message}")]
    Extract { version: String, message: String },

    /// A tarball entry attempted to escape the extraction directory.
    #[error("tarball contains path traversal entry \"{entry_path}\" that escapes {dest}")]
    PathTraversal { entry_path: String, dest: String },

    /// The installed toolchain version does not match the expected version.
    #[error("expected Kotlin/Native {expected} but found {actual}")]
    VersionMismatch { expected: String, actual: String },

    /// The managed toolchain installation is corrupt or incomplete.
    #[error("corrupt toolchain at {path} — run `konvoy toolchain install {version}` to reinstall")]
    CorruptToolchain { path: PathBuf, version: String },

    /// Failed to install or locate the bundled JRE.
    #[error("JRE installation failed: {message}")]
    JreInstall { message: String },

    /// A filesystem operation failed during toolchain management.
    ///
    /// Kept for raw `std::fs` calls (read_dir iteration, metadata, set_permissions,
    /// tempfile creation, etc.) that have no konvoy-util wrapper.
    #[error("cannot access {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}
