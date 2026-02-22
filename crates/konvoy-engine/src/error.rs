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

    /// A dependency cycle was detected.
    #[error("dependency cycle detected: {cycle}")]
    DependencyCycle { cycle: String },

    /// A dependency project was not found on disk.
    #[error("dependency `{name}` not found at {path}")]
    DependencyNotFound { name: String, path: String },

    /// A dependency is not a library project.
    #[error("dependency `{name}` must have kind = \"lib\" (found kind = \"bin\" at {path})")]
    DependencyNotLib { name: String, path: String },

    /// A dependency uses a different Kotlin version than the root project.
    #[error("dependency `{name}` requires Kotlin {dep_version}, but root project requires {root_version}")]
    DependencyToolchainMismatch {
        name: String,
        dep_version: String,
        root_version: String,
    },

    /// A dependency path escapes the project tree.
    #[error("dependency `{name}` path escapes the project tree — resolved to {path}; use a relative path within the workspace")]
    DependencyPathEscape { name: String, path: String },

    /// A tarball hash in the lockfile does not match the freshly downloaded hash.
    #[error("{kind} tarball hash mismatch — expected {expected}, got {actual}; re-run with --force to override")]
    TarballHashMismatch {
        kind: String,
        expected: String,
        actual: String,
    },
}
