//! Managed external tools: download, cache, SHA-verify, and run a versioned
//! artifact from `~/.konvoy/tools/<id>/<version>/`. Shared by the detekt linter
//! and the codegen tools (e.g. Fabrikt) through the same
//! [`ensure`](ManagedToolSpec::ensure) + [`run`](ManagedToolSpec::run) interface.
//!
//! Two orthogonal axes are modeled as closed enums, each dispatched by an
//! exhaustive `match`: [`ToolSource`] is *where* the artifact comes from (Maven vs
//! a direct URL), and [`ToolRuntime`] is *how* it is launched (a JVM JAR via the
//! managed JRE, or a native binary exec'd directly). They are independent — a
//! `DirectUrl` artifact can be a JAR (detekt) or a native binary, and a `Maven`
//! artifact is a JAR today (Fabrikt) but nothing here assumes the JVM.
//!
//! Download error mapping (`DetektDownload` vs `CodegenDownload`, …) and lockfile
//! pinning stay with the caller: [`ensure`](ManagedToolSpec::ensure) returns the
//! raw [`UtilError`] so each caller can map it via `error::map_artifact_download_err`
//! and persist the hash wherever its lockfile section lives. Execution, by
//! contrast, is uniform across tools: [`run`](ManagedToolSpec::run) reports only
//! whether the tool *could be executed* (a shared [`EngineError::ToolExecFailed`])
//! and hands back a [`ToolOutput`] whose `success` flag the caller interprets —
//! a generator treats a failed run as an error, a linter as "issues found".

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use konvoy_util::error::UtilError;
use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

use crate::error::EngineError;

/// Where a managed tool's artifact is fetched from. Orthogonal to [`ToolRuntime`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    /// A Maven Central artifact addressed by coordinate (e.g. Fabrikt).
    Maven(MavenCoordinate),
    /// A direct release URL with a fixed filename (e.g. detekt's GitHub release).
    DirectUrl { url: String, filename: String },
}

/// How a managed tool is launched once downloaded. Orthogonal to [`ToolSource`]:
/// a `DirectUrl` artifact may be a JVM JAR (detekt) or a native binary, and a
/// `Maven` artifact is a JAR today (Fabrikt) but the runtime axis does not assume it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolRuntime {
    /// Launched as `<jre_home>/bin/java -jar <artifact> <args>` with
    /// `JAVA_HOME=<jre_home>`. Requires a managed JRE supplied to [`run`](ManagedToolSpec::run).
    #[default]
    Jvm,
    /// The downloaded artifact is itself an executable; exec it directly, no JRE.
    Native,
}

/// Captured result of running a managed tool (see [`ManagedToolSpec::run`]).
///
/// `success` is a plain pass/fail flag derived from the process exit status; the
/// raw exit code is deliberately not surfaced. Callers decide what a failed run
/// means — a code generator treats `!success` as an error, while a linter treats
/// it as "issues were found".
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Captured standard output (lossy UTF-8).
    pub stdout: String,
    /// Captured standard error (lossy UTF-8).
    pub stderr: String,
    /// Whether the process exited successfully (exit status 0).
    pub success: bool,
}

/// A managed external tool downloaded into `~/.konvoy/tools/<id>/<version>/`.
///
/// Fields are private so [`maven_jar`](Self::maven_jar) / [`direct_url`](Self::direct_url)
/// are the only construction path. The constructors do not validate; the traversal
/// guard that rejects a `version`/`filename` which would escape the tools dir runs
/// in the methods that touch the filesystem — [`ensure`](Self::ensure) (download)
/// and [`run`](Self::run) (exec). The runtime defaults to [`ToolRuntime::Jvm`];
/// chain [`native`](Self::native) for a native tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedToolSpec {
    /// Stable lockfile/cache id, e.g. `"fabrikt"` or `"detekt"`.
    id: String,
    /// Human-readable name used in the download progress bar. (Diagnostics use the
    /// stable `id` instead — see [`ToolExecFailed`](crate::error::EngineError::ToolExecFailed).)
    display_name: String,
    /// Tool version.
    version: String,
    /// Where the artifact comes from.
    source: ToolSource,
    /// How the downloaded artifact is launched.
    runtime: ToolRuntime,
}

impl ManagedToolSpec {
    /// A managed Maven Central JAR; the version is taken from the coordinate.
    /// Defaults to the [`ToolRuntime::Jvm`] runtime.
    #[must_use]
    pub fn maven_jar(id: &str, display_name: &str, coordinate: MavenCoordinate) -> Self {
        Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            version: coordinate.version.clone(),
            source: ToolSource::Maven(coordinate),
            runtime: ToolRuntime::Jvm,
        }
    }

    /// A managed artifact fetched from a direct release URL with a fixed filename.
    /// Defaults to the [`ToolRuntime::Jvm`] runtime; chain [`native`](Self::native)
    /// for a native binary.
    #[must_use]
    pub fn direct_url(
        id: &str,
        display_name: &str,
        version: &str,
        url: String,
        filename: String,
    ) -> Self {
        Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            version: version.to_owned(),
            source: ToolSource::DirectUrl { url, filename },
            runtime: ToolRuntime::Jvm,
        }
    }

    /// Mark this tool as a native binary: [`run`](Self::run) execs the downloaded
    /// artifact directly with no JRE. (The on-disk `filename` is used verbatim as
    /// the executable.)
    #[must_use]
    pub fn native(mut self) -> Self {
        self.runtime = ToolRuntime::Native;
        self
    }

    /// The stable lockfile/cache id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// How this tool is launched.
    #[must_use]
    pub fn runtime(&self) -> ToolRuntime {
        self.runtime
    }

    /// The tool version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The artifact filename on disk.
    #[must_use]
    pub fn filename(&self) -> String {
        match &self.source {
            ToolSource::Maven(coordinate) => coordinate.filename(),
            ToolSource::DirectUrl { filename, .. } => filename.clone(),
        }
    }

    /// The download URL for this tool.
    #[must_use]
    pub fn download_url(&self) -> String {
        match &self.source {
            ToolSource::Maven(coordinate) => coordinate.to_url(MAVEN_CENTRAL),
            ToolSource::DirectUrl { url, .. } => url.clone(),
        }
    }

    /// The managed artifact path under `~/.konvoy/tools/<id>/<version>/`.
    ///
    /// This is a pure path computation (read-only callers like `is_installed`
    /// rely on it not erroring on a malformed version). The traversal guard lives
    /// in [`ensure`](Self::ensure), the only method that *writes* — see `validate`.
    ///
    /// # Errors
    /// Returns an error if the Konvoy home directory cannot be resolved.
    pub fn artifact_path(&self) -> Result<PathBuf, UtilError> {
        Ok(konvoy_util::fs::konvoy_home()?
            .join("tools")
            .join(&self.id)
            .join(&self.version)
            .join(self.filename()))
    }

    /// Whether the artifact already exists locally.
    ///
    /// # Errors
    /// Returns an error if the Konvoy home directory cannot be resolved.
    pub fn is_installed(&self) -> Result<bool, UtilError> {
        Ok(self.artifact_path()?.exists())
    }

    /// Download (or verify a cached) artifact, returning `(path, sha256)`.
    ///
    /// Domain error mapping is the caller's job: map the returned `UtilError`
    /// via `error::map_artifact_download_err`.
    ///
    /// # Errors
    /// Returns an error if the id/version/filename is unsafe, the artifact cannot
    /// be downloaded, or the expected SHA-256 does not match.
    pub fn ensure(&self, expected_sha256: Option<&str>) -> Result<(PathBuf, String), UtilError> {
        // Validate here (the only method that writes), so a traversal-laden
        // version/filename can never escape the tools dir during a download.
        self.validate()?;

        let artifact_path = self.artifact_path()?;
        // Only show a download bar when the JAR isn't already cached — a cached
        // re-verify completes in milliseconds and the flash is more noise than
        // information.
        let progress = (!artifact_path.exists()).then(|| {
            konvoy_util::progress::new_download_bar(format!(
                "{} {}",
                self.display_name, self.version
            ))
        });
        // Pass the validated `id` (not the free-text display_name) as the fetch
        // label — download_artifact embeds it in a temp filename, so a separator
        // in display_name would point the temp path at a missing directory.
        let result = konvoy_util::progress::fetch(
            &self.download_url(),
            &artifact_path,
            expected_sha256,
            &self.id,
            progress.as_ref(),
        )?;
        if progress.is_some() {
            eprintln!();
        }

        Ok((result.path, result.sha256))
    }

    /// Run this tool's downloaded artifact, capturing stdout/stderr.
    ///
    /// The artifact must already be present — call [`ensure`](Self::ensure) first.
    /// The launch mechanism depends on [`runtime`](Self::runtime):
    /// - [`ToolRuntime::Jvm`] runs `<jre_home>/bin/java -jar <artifact> <args>` with
    ///   `JAVA_HOME=<jre_home>`, so `jre_home` must be `Some`.
    /// - [`ToolRuntime::Native`] execs the artifact directly and ignores `jre_home`.
    ///
    /// Reports only *whether the tool could be executed*: a non-zero exit is
    /// surfaced as `ToolOutput::success == false`, not an error, so each caller can
    /// interpret it (a generator treats it as failure, a linter as "issues found").
    /// The raw exit code is intentionally not exposed.
    ///
    /// # Errors
    /// Returns [`EngineError::ToolExecFailed`] if the artifact path cannot be
    /// resolved, a JVM tool is given no `jre_home` or the managed `java` is missing,
    /// a native binary is missing, or the process cannot be spawned.
    pub fn run(
        &self,
        jre_home: Option<&Path>,
        args: &[OsString],
        verbose: bool,
    ) -> Result<ToolOutput, EngineError> {
        // run() is an exec entry point, so — like ensure(), the download entry
        // point — it validates first: a version/filename with `..` or a separator
        // must never resolve a traversal path and exec whatever is there, even if
        // ensure() was bypassed (e.g. a --locked path that only saw is_installed()).
        self.validate()
            .map_err(|e| self.exec_failed(format!("invalid tool spec: {e}")))?;

        // Map to ToolExecFailed (not the bare Util variant) so run()'s whole error
        // surface is tool-attributed, matching this method's documented contract.
        let artifact = self
            .artifact_path()
            .map_err(|e| self.exec_failed(format!("cannot resolve tool path: {e}")))?;
        let missing_artifact = || {
            self.exec_failed(format!(
                "tool artifact not found at {} — re-run the command to download it",
                artifact.display()
            ))
        };

        // Build the Command per runtime; the spawn/capture/verbose/success tail is
        // shared (see `capture`). Each arm does its own no-spawn pre-checks so a
        // missing program is an actionable ToolExecFailed, never a raw ENOENT or a
        // confusing JVM "Unable to access jarfile".
        let cmd = match self.runtime {
            ToolRuntime::Jvm => {
                // The JRE is JVM-only; a Jvm tool without one is a wiring/setup
                // error, surfaced as the same no-spawn ToolExecFailed as missing java.
                let jre = jre_home.ok_or_else(|| {
                    self.exec_failed(
                        "this tool runs on the managed JRE but no JRE was provided — \
                         run `konvoy toolchain install`"
                            .to_owned(),
                    )
                })?;
                let java = jre.join("bin").join("java");
                if !java.exists() {
                    return Err(self.exec_failed(format!(
                        "java not found at {} — run `konvoy toolchain install` to reinstall the managed JRE",
                        java.display()
                    )));
                }
                if !artifact.exists() {
                    return Err(missing_artifact());
                }
                let mut cmd = Command::new(&java);
                cmd.arg("-jar")
                    .arg(&artifact)
                    .args(args)
                    .env("JAVA_HOME", jre);
                cmd
            }
            ToolRuntime::Native => {
                if !artifact.exists() {
                    return Err(missing_artifact());
                }
                let mut cmd = Command::new(&artifact);
                cmd.args(args);
                cmd
            }
        };

        capture(&self.id, cmd, verbose)
    }

    /// Build an [`EngineError::ToolExecFailed`] for this tool (the `id` is private).
    fn exec_failed(&self, message: String) -> EngineError {
        EngineError::ToolExecFailed {
            tool: self.id.clone(),
            message,
        }
    }

    /// Reject any component that could escape `~/.konvoy/tools/<id>/<version>/`.
    /// `validate_identifier` allows `[A-Za-z0-9._-]` but rejects `..`, covering
    /// the id, version, and the on-disk filename (Maven `artifact-version.ext`
    /// or the caller-supplied direct-URL filename).
    fn validate(&self) -> Result<(), UtilError> {
        konvoy_util::artifact::validate_identifier(&self.id)?;
        konvoy_util::artifact::validate_identifier(&self.version)?;
        konvoy_util::artifact::validate_identifier(&self.filename())?;
        if let ToolSource::Maven(coordinate) = &self.source {
            konvoy_util::artifact::validate_identifier(&coordinate.group_id)?;
            konvoy_util::artifact::validate_identifier(&coordinate.artifact_id)?;
        }
        Ok(())
    }
}

/// Single owner of the spawn + capture + verbose-echo + success contract shared by
/// every runtime (mirrors konanc's build/execute split). Turns a configured
/// `Command` into a [`ToolOutput`]; the raw exit code is never surfaced, and a
/// spawn failure becomes a [`EngineError::ToolExecFailed`].
fn capture(tool_id: &str, mut cmd: Command, verbose: bool) -> Result<ToolOutput, EngineError> {
    let output = cmd.output().map_err(|e| EngineError::ToolExecFailed {
        tool: tool_id.to_owned(),
        message: e.to_string(),
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if verbose {
        if !stdout.is_empty() {
            eprintln!("{stdout}");
        }
        if !stderr.is_empty() {
            eprintln!("{stderr}");
        }
    }

    Ok(ToolOutput {
        stdout,
        stderr,
        success: output.status.success(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_jvm() -> ManagedToolSpec {
        ManagedToolSpec::direct_url(
            "demo",
            "Demo",
            "1.0.0",
            "https://example.invalid/demo.jar".to_owned(),
            "demo-1.0.0.jar".to_owned(),
        )
    }

    /// A JVM tool with a `jre_home` that has no `java` must fail with the shared
    /// `ToolExecFailed` (not a leaked exit code or a panic) — and never spawn.
    #[test]
    fn run_errors_when_java_missing() {
        let tool = demo_jvm();
        // A jre_home that does not contain bin/java — run must bail before spawning.
        let bogus_jre = Path::new("/konvoy/definitely/not/a/jre");

        match tool.run(Some(bogus_jre), &[], false) {
            Err(EngineError::ToolExecFailed { tool, message }) => {
                assert_eq!(tool, "demo");
                assert!(
                    message.contains("java not found"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected ToolExecFailed, got {other:?}"),
        }
    }

    /// A JVM tool given no JRE at all bails with `ToolExecFailed` before spawning.
    #[test]
    fn run_errors_when_jvm_tool_has_no_jre() {
        match demo_jvm().run(None, &[], false) {
            Err(EngineError::ToolExecFailed { tool, message }) => {
                assert_eq!(tool, "demo");
                assert!(message.contains("no JRE"), "unexpected message: {message}");
            }
            other => panic!("expected ToolExecFailed, got {other:?}"),
        }
    }

    /// A native tool whose binary is absent bails with `ToolExecFailed` before
    /// spawning, and ignores `jre_home` entirely (passing `None` is fine).
    #[test]
    fn run_native_errors_when_binary_missing() {
        let tool = ManagedToolSpec::direct_url(
            "demo-native",
            "DemoNative",
            "1.0.0",
            "https://example.invalid/demo".to_owned(),
            "demo-1.0.0".to_owned(),
        )
        .native();
        assert_eq!(tool.runtime(), ToolRuntime::Native);

        match tool.run(None, &[], false) {
            Err(EngineError::ToolExecFailed { tool, message }) => {
                assert_eq!(tool, "demo-native");
                assert!(
                    message.contains("artifact not found"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected ToolExecFailed, got {other:?}"),
        }
    }

    /// run() validates the spec before resolving/exec'ing a path, so a traversal
    /// version (e.g. `..`) is rejected up front rather than exec'ing an arbitrary
    /// file — even when ensure() (which also validates) was never called.
    #[test]
    fn run_rejects_traversal_version() {
        let tool = ManagedToolSpec::direct_url(
            "demo",
            "Demo",
            "..",
            "https://example.invalid/demo.jar".to_owned(),
            "demo.jar".to_owned(),
        )
        .native();
        match tool.run(None, &[], false) {
            Err(EngineError::ToolExecFailed { tool, message }) => {
                assert_eq!(tool, "demo");
                assert!(
                    message.contains("invalid tool spec"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected ToolExecFailed, got {other:?}"),
        }
    }

    /// A `..` confined to the on-disk filename (every char valid, but the `..`
    /// substring) must also be rejected — not just a bad version. Guards the
    /// `filename()` branch of validate().
    #[test]
    fn run_rejects_traversal_filename() {
        let tool = ManagedToolSpec::direct_url(
            "demo",
            "Demo",
            "1.0.0",
            "https://example.invalid/x".to_owned(),
            "../evil.jar".to_owned(),
        )
        .native();
        match tool.run(None, &[], false) {
            Err(EngineError::ToolExecFailed { message, .. }) => {
                assert!(
                    message.contains("invalid tool spec"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected ToolExecFailed, got {other:?}"),
        }
    }

    /// capture() owns the spawn + output + success contract that every run() arm
    /// funnels into; the run() tests only hit pre-spawn errors, so exercise the
    /// real path: a process that exits 0, with output captured on both streams and
    /// the success flag set — without leaking the raw exit code.
    #[test]
    fn capture_reports_success_and_captures_both_streams() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf out; printf err >&2; exit 0"]);
        let out = capture("demo", cmd, false).expect("spawn should succeed");
        assert!(out.success);
        assert_eq!(out.stdout, "out");
        assert_eq!(out.stderr, "err");
    }

    /// A non-zero exit becomes `success == false` (still `Ok`, since the process
    /// *ran*) — the failure is a flag, not an error, and the code itself is hidden.
    #[test]
    fn capture_reports_failure_on_nonzero_exit() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 7"]);
        let out = capture("demo", cmd, false).expect("spawn should succeed");
        assert!(!out.success);
    }

    /// A program that cannot be spawned at all is an error (not a `success=false`
    /// ToolOutput), attributed to the tool.
    #[test]
    fn capture_errors_when_program_missing() {
        let cmd = Command::new("/konvoy/definitely/not/a/program");
        match capture("demo", cmd, false) {
            Err(EngineError::ToolExecFailed { tool, .. }) => assert_eq!(tool, "demo"),
            other => panic!("expected ToolExecFailed, got {other:?}"),
        }
    }
}
