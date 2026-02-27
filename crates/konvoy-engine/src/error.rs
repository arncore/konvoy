//! Error types for konvoy-engine.

/// Errors produced by engine operations.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
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
    Konanc(#[from] konvoy_konanc::error::KonancError),

    /// A target resolution failed.
    #[error("{0}")]
    Target(#[from] konvoy_targets::TargetError),

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
    #[error("{kind} tarball hash mismatch — expected {expected}, got {actual}; this may indicate a tampered or corrupted download — re-run with --force to re-download, or verify the hash against the upstream release")]
    TarballHashMismatch {
        kind: String,
        expected: String,
        actual: String,
    },

    /// A dependency source hash does not match the lockfile (in --locked mode).
    #[error("dependency `{name}` source hash mismatch — locked: {expected}, current: {actual}; this may indicate unexpected source changes — remove --locked to allow lockfile updates, or verify the dependency sources have not been tampered with")]
    DependencyHashMismatch {
        name: String,
        expected: String,
        actual: String,
    },

    /// The lockfile would need updating but --locked mode prevents it.
    #[error(
        "lockfile is out of date and --locked prevents updates; run without --locked to update"
    )]
    LockfileUpdateRequired,

    /// No test source files found.
    #[error("no test source files found in {dir} — create test files in src/test/ using kotlin.test annotations")]
    NoTestSources { dir: String },

    /// Failed to download detekt.
    #[error("cannot download detekt {version}: {message}")]
    DetektDownload { version: String, message: String },

    /// Failed to run detekt.
    #[error("cannot run detekt: {message}")]
    DetektExec { message: String },

    /// No JRE available to run detekt.
    #[error("jre not available for running detekt — run `konvoy toolchain install` first")]
    DetektNoJre,

    /// Detekt jar hash mismatch.
    #[error("detekt {version} jar hash mismatch — expected {expected}, got {actual}; this may indicate a tampered or corrupted download — delete ~/.konvoy/tools/detekt/{version}/ and re-run `konvoy lint` to re-download, or verify the hash at the detekt release page")]
    DetektHashMismatch {
        version: String,
        expected: String,
        actual: String,
    },

    /// Lint not configured.
    #[error("detekt not configured — add `detekt = \"1.23.7\"` to [toolchain] in konvoy.toml")]
    LintNotConfigured,

    /// The project name supplied to `konvoy init` is invalid.
    #[error("invalid project name \"{name}\": {reason}")]
    InvalidProjectName { name: String, reason: String },

    /// An explicit config file was not found on disk.
    #[error("config file not found: {path} — check the --config path or create the file")]
    ConfigNotFound { path: String },

    /// An unknown plugin was referenced in the manifest.
    #[error("unknown plugin `{name}` — available plugins: {available}")]
    UnknownPlugin { name: String, available: String },

    /// A plugin module name is invalid.
    #[error("unknown module `{module}` for plugin `{plugin}` — available modules: {available}")]
    UnknownPluginModule {
        plugin: String,
        module: String,
        available: String,
    },

    /// Plugin configuration is invalid.
    #[error("invalid plugin `{name}` configuration: {reason}")]
    InvalidPluginConfig { name: String, reason: String },

    /// A library descriptor is invalid.
    #[error("invalid library descriptor `{name}`: {reason}")]
    InvalidLibraryDescriptor { name: String, reason: String },

    /// An unknown library was referenced in the manifest.
    #[error("unknown library `{name}` — available libraries: {available}")]
    UnknownLibrary { name: String, available: String },

    /// Plugin artifact download failed.
    #[error("cannot download plugin `{name}` artifact: {message}")]
    PluginDownload { name: String, message: String },

    /// Plugin artifact hash mismatch.
    #[error("plugin `{name}` artifact hash mismatch for {artifact} — expected {expected}, got {actual}; delete the cached artifact and re-run to re-download")]
    PluginHashMismatch {
        name: String,
        artifact: String,
        expected: String,
        actual: String,
    },

    /// A Maven library artifact download failed.
    #[error("cannot download library `{name}` from {url}: {message}")]
    LibraryDownloadFailed {
        name: String,
        url: String,
        message: String,
    },

    /// A dependency is not in the lockfile (user should run `konvoy update`).
    #[error("dependency `{name}` not in lockfile — run `konvoy update` to resolve")]
    MissingLockfileEntry { name: String },

    /// A target hash is missing from the lockfile for a Maven dependency.
    #[error("no hash for target `{target}` in lockfile for dependency `{name}` — run `konvoy update` to resolve")]
    MissingTargetHash { name: String, target: String },

    /// A library klib hash mismatch was detected after download.
    #[error("library `{name}` klib hash mismatch — expected {expected}, got {actual}; run `konvoy update` to refresh")]
    LibraryHashMismatch {
        name: String,
        expected: String,
        actual: String,
    },
}
