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
    #[error("{0}")]
    Lockfile(#[from] konvoy_config::lockfile::LockfileError),

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

    /// Plugin configuration is invalid.
    #[error("invalid plugin `{name}` configuration: {reason}")]
    InvalidPluginConfig { name: String, reason: String },

    /// Plugin artifact download failed.
    #[error("cannot download plugin `{name}` artifact: {message}")]
    PluginDownload { name: String, message: String },

    /// Plugin artifact hash mismatch.
    #[error("plugin `{name}` hash mismatch — expected {expected}, got {actual}; delete the cached artifact and re-run to re-download")]
    PluginHashMismatch {
        name: String,
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

    /// A library klib hash mismatch was detected after a fresh download.
    ///
    /// Unlike `ArtifactHashMismatch` (cached file mismatch), this fires when
    /// the just-downloaded content doesn't match the lockfile. This points at
    /// a network-level issue (MITM, CDN corruption) or a stale lockfile.
    #[error("library `{name}` hash mismatch after download\n  expected: {expected}\n  got:      {actual}\n\n  This can happen if:\n    - the download was intercepted or corrupted in transit\n    - the lockfile hashes are stale (e.g. the upstream artifact was republished)\n\n  To fix: run `konvoy update` to re-resolve and re-hash all dependencies.\n  If this keeps happening, check your network connection for interference.")]
    LibraryHashMismatch {
        name: String,
        expected: String,
        actual: String,
    },

    /// Two dependency paths require different versions of the same Maven artifact.
    #[error("version conflict for '{maven}'\n{details}\n  hint: add an explicit version in konvoy.toml:\n    {hint_name} = {{ maven = \"{maven}\", version = \"{hint_version}\" }}")]
    MavenVersionConflict {
        maven: String,
        details: String,
        hint_name: String,
        hint_version: String,
    },

    /// A cycle was detected during Maven transitive dependency resolution.
    #[error("maven dependency cycle detected: {cycle} — remove one of these dependencies from konvoy.toml or file an issue upstream")]
    MavenDependencyCycle { cycle: String },
}

/// Map a `UtilError` from artifact download/verify to an `EngineError`.
///
/// Shared helper used by detekt, plugin, and library artifact pipelines.
/// `make_download` builds the download-failure variant from `(label, message)`;
/// `make_hash` builds the hash-mismatch variant from `(label, expected, actual)`.
/// Other `UtilError` variants pass through as `EngineError::Util`.
pub(crate) fn map_artifact_download_err(
    label: &str,
    util_err: konvoy_util::error::UtilError,
    make_download: impl FnOnce(String, String) -> EngineError,
    make_hash: impl FnOnce(String, String, String) -> EngineError,
) -> EngineError {
    match util_err {
        konvoy_util::error::UtilError::Download { message } => {
            make_download(label.to_owned(), message)
        }
        konvoy_util::error::UtilError::ArtifactHashMismatch {
            expected, actual, ..
        } => make_hash(label.to_owned(), expected, actual),
        other => EngineError::Util(other),
    }
}
