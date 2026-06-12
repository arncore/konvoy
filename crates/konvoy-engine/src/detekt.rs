//! Detekt tool management: download, invocation, and output parsing.
//!
//! Downloads `detekt-cli` fat JARs from GitHub releases and runs them
//! against Kotlin source files using the JRE bundled with managed toolchains.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::EngineError;
use crate::managed_tool::ManagedToolSpec;

/// Map a `UtilError` from artifact operations to the corresponding `EngineError`.
fn map_download_err(version: &str, e: konvoy_util::error::UtilError) -> EngineError {
    crate::error::map_artifact_download_err(
        version,
        e,
        |version, message| EngineError::DetektDownload { version, message },
        |version, expected, actual| EngineError::DetektHashMismatch {
            version,
            expected,
            actual,
        },
    )
}

/// Options for the `lint` command.
#[derive(Debug, Clone)]
pub struct LintOptions {
    /// Whether to show raw detekt output.
    pub verbose: bool,
    /// Optional path to a custom detekt configuration file.
    pub config: Option<PathBuf>,
}

/// Result of running detekt.
#[derive(Debug, Clone)]
pub struct LintResult {
    /// Whether detekt exited successfully (no findings).
    pub success: bool,
    /// Parsed diagnostics from detekt output.
    pub diagnostics: Vec<DetektDiagnostic>,
    /// Raw stderr output from detekt.
    pub raw_output: String,
    /// Number of findings.
    pub finding_count: usize,
}

/// A single diagnostic finding from detekt.
#[derive(Debug, Clone)]
pub struct DetektDiagnostic {
    /// The rule name (e.g. "MagicNumber").
    pub rule: String,
    /// The diagnostic message.
    pub message: String,
    /// Source file path, if available.
    pub file: Option<String>,
    /// Line number, if available.
    pub line: Option<u32>,
}

/// The managed-JAR-tool spec for a detekt version — a GitHub release downloaded
/// into `~/.konvoy/tools/detekt/<version>/`.
fn detekt_tool(version: &str) -> ManagedToolSpec {
    ManagedToolSpec::direct_url(
        "detekt",
        "detekt",
        version,
        detekt_download_url(version),
        format!("detekt-cli-{version}-all.jar"),
    )
}

/// Return the path to the detekt-cli JAR for a specific version.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn detekt_jar_path(version: &str) -> Result<PathBuf, EngineError> {
    detekt_tool(version)
        .artifact_path()
        .map_err(EngineError::from)
}

/// Construct the download URL for a detekt-cli release.
pub(crate) fn detekt_download_url(version: &str) -> String {
    format!("https://github.com/detekt/detekt/releases/download/v{version}/detekt-cli-{version}-all.jar")
}

/// Check if detekt is already downloaded for a given version.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn is_installed(version: &str) -> Result<bool, EngineError> {
    detekt_tool(version)
        .is_installed()
        .map_err(EngineError::from)
}

/// Download detekt-cli if not already present, returning the path to the JAR.
///
/// If `expected_sha256` is `Some`, the downloaded (or existing) JAR is verified
/// against it. On first download, returns the computed SHA-256 so the caller
/// can persist it in the lockfile.
///
/// # Errors
/// Returns an error if the download fails, the hash doesn't match, or the
/// home directory cannot be determined.
pub fn ensure_detekt(
    version: &str,
    expected_sha256: Option<&str>,
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<(PathBuf, String), EngineError> {
    // Use the same `validate_identifier` the spec uses (it rejects `..`), so a
    // traversal-laden version yields this actionable, detekt-branded message
    // rather than falling through to the generic download-error mapping.
    konvoy_util::artifact::validate_identifier(version).map_err(|_| EngineError::DetektDownload {
        version: version.to_owned(),
        message: format!(
            "invalid detekt version \"{version}\" — only alphanumeric characters, dots, hyphens, and underscores are allowed, and it cannot be `..`"
        ),
    })?;

    resolver
        .ensure_managed_tool(&detekt_tool(version), expected_sha256)
        .map_err(|e| map_download_err(version, e))
}

/// Resolve the expected detekt hash from the lockfile.
///
/// Returns `None` if the lockfile has no hash or the pinned version doesn't
/// match `detekt_version` (stale entry).
fn resolve_lockfile_hash<'a>(
    lockfile: &'a konvoy_config::lockfile::Lockfile,
    detekt_version: &str,
) -> Option<&'a str> {
    let tc = lockfile.toolchain.as_ref()?;
    let pinned_version = tc.detekt_version.as_deref()?;
    if pinned_version == detekt_version {
        tc.detekt_jar_sha256.as_deref()
    } else {
        None
    }
}

/// Persist the detekt version and JAR hash into the lockfile.
///
/// # Errors
/// Returns an error if the lockfile cannot be written.
fn persist_detekt_hash(
    lockfile_path: &Path,
    lockfile: konvoy_config::lockfile::Lockfile,
    kotlin_version: &str,
    detekt_version: &str,
    hash: String,
) -> Result<(), EngineError> {
    let mut updated = lockfile;
    if let Some(ref mut tc) = updated.toolchain {
        tc.detekt_version = Some(detekt_version.to_owned());
        tc.detekt_jar_sha256 = Some(hash);
    } else {
        updated.toolchain = Some(konvoy_config::lockfile::ToolchainLock {
            konanc_version: kotlin_version.to_owned(),
            konanc_tarball_sha256: None,
            jre_tarball_sha256: None,
            detekt_version: Some(detekt_version.to_owned()),
            detekt_jar_sha256: Some(hash),
        });
    }
    updated.write_to(lockfile_path)?;
    Ok(())
}

/// Resolve the managed Kotlin toolchain's JRE home (which contains `bin/java`).
///
/// Auto-installs the toolchain if it is not yet present. Under `--offline` a
/// missing toolchain is a hard error (no network); under `--locked` the toolchain
/// is still installed on demand (downloading a pinned artifact is allowed). The
/// actual `java` invocation is delegated to [`ManagedToolSpec::run`], which
/// derives the binary from this home.
fn resolve_jre_home(
    kotlin_version: &str,
    lockfile: &konvoy_config::lockfile::Lockfile,
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<PathBuf, EngineError> {
    if !konvoy_konanc::toolchain::is_installed(kotlin_version)? {
        // The detekt JRE rides along with the managed toolchain. If installing
        // would download any missing toolchain artifact under --locked, the
        // relevant tarball hash must already be pinned in konvoy.lock.
        let has_pin = crate::build::has_required_toolchain_artifact_pins(lockfile, kotlin_version)?;
        resolver.resolve_artifact(has_pin, false, || EngineError::DetektJreOffline {
            version: kotlin_version.to_owned(),
        })?;
        eprintln!("    Installing Kotlin/Native {kotlin_version} (for JRE)...");
        resolver.install_toolchain(kotlin_version)?;
    }

    let jre_home = konvoy_konanc::toolchain::jre_home_path(kotlin_version)?;

    if !jre_home.join("bin").join("java").exists() {
        return Err(EngineError::DetektNoJre);
    }

    Ok(jre_home)
}

/// Resolve the detekt config file path.
///
/// If `--config` was passed, resolve it relative to the project root and
/// verify the file exists (returning an error if it does not).
/// Otherwise, use `detekt.yml` in the project root if it exists.
fn resolve_config(root: &Path, explicit: Option<&Path>) -> Result<Option<PathBuf>, EngineError> {
    if let Some(cfg) = explicit {
        let resolved = if cfg.is_relative() {
            root.join(cfg)
        } else {
            cfg.to_path_buf()
        };
        if !resolved.exists() {
            return Err(EngineError::ConfigNotFound {
                path: resolved.display().to_string(),
            });
        }
        Ok(Some(resolved))
    } else {
        let default_config = root.join("detekt.yml");
        if default_config.exists() {
            Ok(Some(default_config))
        } else {
            Ok(None)
        }
    }
}

/// Execute the detekt process and build a `LintResult` from its output.
fn run_detekt_process(
    jre_home: &Path,
    src_dir: &Path,
    config_path: Option<&Path>,
    detekt_version: &str,
    verbose: bool,
) -> Result<LintResult, EngineError> {
    let mut args = vec![OsString::from("--input"), src_dir.as_os_str().to_owned()];

    if let Some(cfg) = config_path {
        args.push(OsString::from("--config"));
        args.push(cfg.as_os_str().to_owned());
        args.push(OsString::from("--build-upon-default-config"));
    }

    eprintln!("    Linting with detekt {detekt_version}...");

    let output = detekt_tool(detekt_version).run(Some(jre_home), &args, verbose)?;

    // Separate the streams with a newline: detekt's findings are line-oriented and
    // `parse_detekt_output` scans per line, so concatenating directly would fuse a
    // trailing (newline-less) stdout line onto the first stderr line and corrupt
    // that finding's parsed `file` field.
    let raw_output = format!("{}\n{}", output.stdout, output.stderr);
    let diagnostics = parse_detekt_output(&raw_output);
    let finding_count = diagnostics.len();

    // `success` is true only when detekt exited 0 (no findings). NOTE: detekt also
    // exits non-zero for real failures (1 = unexpected error, 3 = invalid config),
    // not just for findings (2 = MaxIssuesReached). The shared ToolOutput exposes
    // only a success bool by design, so those collapse together; raw_output still
    // carries detekt's own message for the non-finding cases.
    Ok(LintResult {
        success: output.success,
        diagnostics,
        raw_output,
        finding_count,
    })
}

/// Run detekt on a project's Kotlin source files.
///
/// # Errors
/// Returns an error if the explicitly provided config file does not exist,
/// detekt cannot be downloaded, the JRE is unavailable, or the detekt
/// process fails to execute.
pub fn lint(
    root: &Path,
    options: &LintOptions,
    resolver: crate::common::ArtifactResolver<'_>,
) -> Result<LintResult, EngineError> {
    let manifest = konvoy_config::Manifest::from_path(&root.join("konvoy.toml"))?;

    let detekt_version = manifest
        .toolchain
        .detekt
        .as_deref()
        .ok_or(EngineError::LintNotConfigured)?;

    // Read lockfile and resolve expected hash.
    let lockfile_path = root.join("konvoy.lock");
    let lockfile = konvoy_config::lockfile::Lockfile::from_path(&lockfile_path)?;

    // In --locked mode, run the same fast staleness check `konvoy build` does
    // (issue #295: every command fails for the same reasons). This catches
    // konanc/detekt VERSION drift up-front; the per-artifact gates below only
    // see the detekt JAR's hash pin, not the toolchain version.
    resolver
        .verify_current_lockfile(|| crate::build::check_lockfile_staleness(&manifest, &lockfile))?;

    let expected_hash = resolve_lockfile_hash(&lockfile, detekt_version);

    // Resolve the detekt JAR before any download. has_pin: the lockfile pins
    // this detekt version's JAR hash; is_present: the JAR is already on disk.
    let jar_present = is_installed(detekt_version)?;
    resolver.resolve_artifact(expected_hash.is_some(), jar_present, || {
        EngineError::DetektJarOffline {
            version: detekt_version.to_owned(),
        }
    })?;

    // Ensure detekt jar is available and hash-verified. The path is derived again
    // (from the same spec) inside `run_detekt_process`, so it is discarded here.
    let (_, actual_hash) = ensure_detekt(detekt_version, expected_hash, resolver)?;

    // Persist hash to lockfile if not already stored.
    if expected_hash.is_none() {
        persist_detekt_hash(
            &lockfile_path,
            lockfile.clone(),
            &manifest.toolchain.kotlin,
            detekt_version,
            actual_hash,
        )?;
    }

    // Resolve JRE.
    let jre_home = resolve_jre_home(&manifest.toolchain.kotlin, &lockfile, resolver)?;

    // Check for sources.
    let src_dir = root.join("src");
    if !src_dir.exists() {
        eprintln!("    warning: no Kotlin sources to lint (src/ not found)");
        return Ok(LintResult {
            success: true,
            diagnostics: Vec::new(),
            raw_output: String::new(),
            finding_count: 0,
        });
    }

    // Resolve config and run detekt.
    let config_path = resolve_config(root, options.config.as_deref())?;
    run_detekt_process(
        &jre_home,
        &src_dir,
        config_path.as_deref(),
        detekt_version,
        options.verbose,
    )
}

/// Parse detekt text output into structured diagnostics.
///
/// Detekt 1.23.x default text output format:
/// `file.kt:line:col: message text [RuleName]`
///
/// Also handles the legacy format for robustness:
/// `file.kt:line:col: RuleName - message [detekt.RuleSet]`
pub fn parse_detekt_output(output: &str) -> Vec<DetektDiagnostic> {
    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Try to match real detekt format: file:line:col: message [RuleName]
        // or legacy format:                file:line:col: RuleName - message [detekt.RuleSet]
        if let Some(diag) = parse_detekt_line(trimmed) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

/// Parsed file location from a detekt output line.
struct DetektLocation {
    file: Option<String>,
    line: Option<u32>,
    rest_start: usize,
}

/// Extract file path, line number, and the rest-of-line offset from a detekt line.
///
/// Scans for the first `:<digits>:` pattern (file:line:col: or file:line:).
fn parse_detekt_location(line: &str) -> Option<DetektLocation> {
    let mut file_end = None;
    let mut line_num = None;
    let mut rest_start = 0;

    for (i, ch) in line.char_indices() {
        if ch != ':' || file_end.is_some() {
            continue;
        }
        let remaining = line.get(i + 1..)?;
        let Some(end) = remaining.find(':') else {
            continue;
        };
        let Ok(num) = remaining.get(..end)?.parse::<u32>() else {
            continue;
        };

        file_end = Some(i);
        line_num = Some(num);

        // Skip optional column number after line.
        let after_line = remaining.get(end + 1..)?;
        let col_len = after_line.find(':').and_then(|col_end| {
            after_line
                .get(..col_end)?
                .parse::<u32>()
                .ok()
                .map(|_| col_end + 1)
        });
        rest_start = i + 1 + end + 1 + col_len.unwrap_or(0);
        break;
    }

    let file = file_end.and_then(|end| {
        let f = line.get(..end)?.trim();
        if f.is_empty() {
            None
        } else {
            Some(f.to_owned())
        }
    });

    Some(DetektLocation {
        file,
        line: line_num,
        rest_start,
    })
}

/// Try to parse legacy detekt format: `RuleName - message [detekt.RuleSet]`.
fn try_parse_legacy_format(rest: &str) -> Option<(String, String)> {
    let dash_pos = rest.find(" - ")?;
    let candidate_rule = rest.get(..dash_pos)?.trim();
    // Legacy rule names are single PascalCase identifiers (no spaces, no dots).
    if candidate_rule.is_empty() || candidate_rule.contains(' ') || candidate_rule.contains('.') {
        return None;
    }
    let msg = rest.get(dash_pos + 3..)?;
    // Strip trailing [detekt.RuleSet] if present.
    let msg = match msg.rfind('[') {
        Some(bracket_pos) => msg.get(..bracket_pos)?.trim(),
        None => msg.trim(),
    };
    Some((candidate_rule.to_owned(), msg.to_owned()))
}

/// Try to parse modern detekt format: `message text [RuleName]`.
fn try_parse_modern_format(rest: &str) -> Option<(String, String)> {
    let bracket_open = rest.rfind('[')?;
    let bracket_close = rest.rfind(']')?;
    if bracket_close <= bracket_open {
        return None;
    }
    let rule = rest.get(bracket_open + 1..bracket_close)?.trim();
    if rule.is_empty() {
        return None;
    }
    let message = rest.get(..bracket_open)?.trim();
    if message.is_empty() {
        return None;
    }
    Some((rule.to_owned(), message.to_owned()))
}

/// Parse a single line of detekt output into a diagnostic.
///
/// Handles two formats:
/// - Real detekt 1.23.x: `path/file.kt:line:col: message text [RuleName]`
/// - Legacy format:      `path/file.kt:line:col: RuleName - message [detekt.RuleSet]`
fn parse_detekt_line(line: &str) -> Option<DetektDiagnostic> {
    let loc = parse_detekt_location(line)?;

    let rest = line.get(loc.rest_start..)?.trim();
    if rest.is_empty() {
        return None;
    }

    let (rule, message) =
        try_parse_legacy_format(rest).or_else(|| try_parse_modern_format(rest))?;

    Some(DetektDiagnostic {
        rule,
        message,
        file: loc.file,
        line: loc.line,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{detekt_download_url, detekt_jar_path, parse_detekt_output, resolve_config};

    #[test]
    fn detekt_download_url_format() {
        let url = detekt_download_url("1.23.7");
        assert_eq!(
            url,
            "https://github.com/detekt/detekt/releases/download/v1.23.7/detekt-cli-1.23.7-all.jar"
        );
    }

    #[test]
    fn detekt_jar_path_format() {
        let path = detekt_jar_path("1.23.7").unwrap();
        let s = path.display().to_string();
        assert!(s.contains(".konvoy/tools/detekt/1.23.7"), "path was: {s}");
        assert!(s.contains("detekt-cli-1.23.7-all.jar"), "path was: {s}");
    }

    #[test]
    fn parse_detekt_single_finding() {
        let output = "src/main.kt:3:5: This expression contains a magic number. [MagicNumber]";
        let diags = parse_detekt_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags.first().map(|d| d.file.as_deref()),
            Some(Some("src/main.kt"))
        );
        assert_eq!(diags.first().map(|d| d.line), Some(Some(3)));
        assert_eq!(diags.first().map(|d| d.rule.as_str()), Some("MagicNumber"));
        assert_eq!(
            diags.first().map(|d| d.message.as_str()),
            Some("This expression contains a magic number.")
        );
    }

    #[test]
    fn parse_detekt_without_column() {
        let output = "src/main.kt:10: The method is too long. [LongMethod]";
        let diags = parse_detekt_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags.first().map(|d| d.file.as_deref()),
            Some(Some("src/main.kt"))
        );
        assert_eq!(diags.first().map(|d| d.line), Some(Some(10)));
        assert_eq!(diags.first().map(|d| d.rule.as_str()), Some("LongMethod"));
    }

    #[test]
    fn parse_detekt_multiple_findings() {
        let output = "\
src/main.kt:3:5: Magic number. [MagicNumber]
src/util.kt:20:1: Method too long. [LongMethod]
src/app.kt:5:10: Empty function. [EmptyFunctionBlock]";
        let diags = parse_detekt_output(output);
        assert_eq!(diags.len(), 3);
        assert_eq!(diags.get(0).map(|d| d.rule.as_str()), Some("MagicNumber"));
        assert_eq!(diags.get(1).map(|d| d.rule.as_str()), Some("LongMethod"));
        assert_eq!(
            diags.get(2).map(|d| d.rule.as_str()),
            Some("EmptyFunctionBlock")
        );
    }

    #[test]
    fn parse_detekt_empty_output() {
        let diags = parse_detekt_output("");
        assert!(diags.is_empty());
    }

    #[test]
    fn parse_detekt_non_finding_lines_skipped() {
        let output = "\
detekt finished in 1234ms
Overall debt: 10min
src/main.kt:3:5: Magic number. [MagicNumber]
";
        let diags = parse_detekt_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.first().map(|d| d.rule.as_str()), Some("MagicNumber"));
    }

    #[test]
    fn parse_detekt_legacy_format() {
        // Legacy format: "RuleName - message [detekt.RuleSet]"
        let output = "src/main.kt:5:1: UnusedImport - Unused import detected. [detekt.style]";
        let diags = parse_detekt_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.first().map(|d| d.rule.as_str()), Some("UnusedImport"));
        assert_eq!(
            diags.first().map(|d| d.message.as_str()),
            Some("Unused import detected.")
        );
    }

    #[test]
    fn parse_detekt_legacy_format_without_bracket() {
        // Legacy format without the trailing bracket.
        let output = "src/main.kt:5:1: UnusedImport - Unused import detected.";
        let diags = parse_detekt_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.first().map(|d| d.rule.as_str()), Some("UnusedImport"));
        assert_eq!(
            diags.first().map(|d| d.message.as_str()),
            Some("Unused import detected.")
        );
    }

    #[test]
    fn ensure_detekt_hash_mismatch_on_existing_jar() {
        // Create a fake JAR file at the expected path.
        let version = "99.0.0-test";
        let jar = super::detekt_jar_path(version).unwrap();
        if let Some(parent) = jar.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&jar, b"fake jar content").unwrap();

        // Provide a bogus expected hash — should trigger mismatch.
        let result = super::ensure_detekt(
            version,
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(false),
                crate::common::LockfileManager::new(false),
            ),
        );
        // Clean up before asserting.
        let _ = std::fs::remove_file(&jar);
        let _ = std::fs::remove_dir(jar.parent().unwrap());

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("hash mismatch") || err.contains("Hash"),
            "error was: {err}"
        );
    }

    #[test]
    fn ensure_detekt_accepts_matching_hash_on_existing_jar() {
        // Create a fake JAR and compute its real hash.
        let version = "99.0.1-test";
        let jar = super::detekt_jar_path(version).unwrap();
        if let Some(parent) = jar.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = b"deterministic test content";
        std::fs::write(&jar, content).unwrap();

        let real_hash = konvoy_util::hash::sha256_bytes(content);
        let result = super::ensure_detekt(
            version,
            Some(&real_hash),
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(false),
                crate::common::LockfileManager::new(false),
            ),
        );
        // Clean up.
        let _ = std::fs::remove_file(&jar);
        let _ = std::fs::remove_dir(jar.parent().unwrap());

        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        let (path, hash) = result.unwrap();
        assert_eq!(hash, real_hash);
        assert!(path.display().to_string().contains(version));
    }

    /// A version that is charset-valid but contains `..` (e.g. a `1..2` typo) must
    /// be rejected up front with the curated detekt message — not fall through to a
    /// generic error. Regression guard for the `validate_version` -> `validate_identifier`
    /// fix: the weaker `validate_version` would have let `..` pass (a plain `/` would
    /// be caught by either, so it does not exercise the difference).
    #[test]
    fn ensure_detekt_rejects_dotdot_version() {
        let err = super::ensure_detekt(
            "1..2",
            None,
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(false),
                crate::common::LockfileManager::new(false),
            ),
        )
        .expect_err("a `..` version must be rejected before any download");
        let msg = err.to_string();
        assert!(
            msg.contains("invalid detekt version"),
            "expected the curated detekt message, got: {msg}"
        );
    }

    #[test]
    fn lint_offline_errors_when_toolchain_missing() {
        // --offline is the NEW home for "the JRE's toolchain isn't installed".
        // Pre-install a fake detekt JAR pinned in the lockfile so the JAR gate
        // passes and lint reaches JRE resolution, where the absent toolchain
        // becomes a hard error.
        let detekt_version = "99.0.2-test";
        let jar = super::detekt_jar_path(detekt_version).unwrap();
        std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
        let content = b"fake jar for offline jre test";
        std::fs::write(&jar, content).unwrap();
        let jar_hash = konvoy_util::hash::sha256_bytes(content);

        // Project manifest pins a Kotlin version that is never installed.
        let kotlin_version = "0.0.0-offline-test";
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("konvoy.toml"),
            format!(
                "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{kotlin_version}\"\ndetekt = \"{detekt_version}\"\n"
            ),
        )
        .unwrap();
        let lockfile = konvoy_config::lockfile::Lockfile {
            toolchain: Some(konvoy_config::lockfile::ToolchainLock {
                konanc_version: kotlin_version.to_owned(),
                konanc_tarball_sha256: None,
                jre_tarball_sha256: None,
                detekt_version: Some(detekt_version.to_owned()),
                detekt_jar_sha256: Some(jar_hash),
            }),
            ..Default::default()
        };
        lockfile.write_to(&root.join("konvoy.lock")).unwrap();

        let result = super::lint(
            root,
            &super::LintOptions {
                verbose: false,
                config: None,
            },
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(true),
                crate::common::LockfileManager::new(false),
            ),
        );

        // Clean up the fake JAR before asserting.
        let _ = std::fs::remove_file(&jar);
        let _ = std::fs::remove_dir(jar.parent().unwrap());

        assert!(
            matches!(
                result,
                Err(crate::error::EngineError::DetektJreOffline { .. })
            ),
            "expected DetektJreOffline, got: {result:?}"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("--offline"),
            "error should mention --offline: {err}"
        );
        assert!(
            err.contains(kotlin_version),
            "error should mention the toolchain version: {err}"
        );
    }

    #[test]
    fn lint_offline_errors_when_detekt_jar_missing() {
        // --offline + a pinned-but-not-downloaded detekt JAR: hard error from the
        // JAR gate, before any JRE resolution. The version is never installed, so
        // there is no JAR on disk and nothing to clean up.
        let detekt_version = "99.0.3-offline-absent";
        let kotlin_version = "0.0.0-offline-jar-test";
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("konvoy.toml"),
            format!(
                "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{kotlin_version}\"\ndetekt = \"{detekt_version}\"\n"
            ),
        )
        .unwrap();
        let lockfile = konvoy_config::lockfile::Lockfile {
            toolchain: Some(konvoy_config::lockfile::ToolchainLock {
                konanc_version: kotlin_version.to_owned(),
                konanc_tarball_sha256: None,
                jre_tarball_sha256: None,
                detekt_version: Some(detekt_version.to_owned()),
                detekt_jar_sha256: Some("0".repeat(64)),
            }),
            ..Default::default()
        };
        lockfile.write_to(&root.join("konvoy.lock")).unwrap();

        let result = super::lint(
            root,
            &super::LintOptions {
                verbose: false,
                config: None,
            },
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(true),
                crate::common::LockfileManager::new(false),
            ),
        );

        match result {
            Err(crate::error::EngineError::DetektJarOffline { version }) => {
                assert_eq!(version, detekt_version);
            }
            other => panic!("expected DetektJarOffline, got: {other:?}"),
        }
    }

    #[test]
    fn lint_locked_errors_on_detekt_drift() {
        // --locked's real failure mode is lockfile drift: the manifest configures
        // detekt but the lockfile has no pinned JAR hash, so the JAR gate reports
        // `LockfileUpdateRequired` before any download.
        let detekt_version = "99.0.4-drift";
        let kotlin_version = "0.0.0-drift-test";
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("konvoy.toml"),
            format!(
                "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{kotlin_version}\"\ndetekt = \"{detekt_version}\"\n"
            ),
        )
        .unwrap();
        // Lockfile pins the toolchain but has NO detekt hash → drift under --locked.
        konvoy_config::lockfile::Lockfile::with_toolchain(kotlin_version)
            .write_to(&root.join("konvoy.lock"))
            .unwrap();

        let result = super::lint(
            root,
            &super::LintOptions {
                verbose: false,
                config: None,
            },
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(false),
                crate::common::LockfileManager::new(true),
            ),
        );

        assert!(
            matches!(
                result,
                Err(crate::error::EngineError::LockfileUpdateRequired)
            ),
            "expected LockfileUpdateRequired, got: {result:?}"
        );
    }

    #[test]
    fn lint_locked_errors_on_toolchain_drift() {
        // lint --locked must fail fast on konanc-version drift, matching
        // `build --locked` (issue #295: every command behaves identically).
        // The detekt JAR pin itself is consistent — only the toolchain version
        // disagrees between manifest and lockfile.
        let detekt_version = "99.0.5-tc-drift";
        let manifest_kotlin = "0.0.0-lint-drift";
        let lockfile_kotlin = "9.9.9-stale";
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("konvoy.toml"),
            format!(
                "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{manifest_kotlin}\"\ndetekt = \"{detekt_version}\"\n"
            ),
        )
        .unwrap();
        let lockfile = konvoy_config::lockfile::Lockfile {
            toolchain: Some(konvoy_config::lockfile::ToolchainLock {
                konanc_version: lockfile_kotlin.to_owned(),
                konanc_tarball_sha256: None,
                jre_tarball_sha256: None,
                detekt_version: Some(detekt_version.to_owned()),
                detekt_jar_sha256: Some("0".repeat(64)),
            }),
            ..Default::default()
        };
        lockfile.write_to(&root.join("konvoy.lock")).unwrap();

        let result = super::lint(
            root,
            &super::LintOptions {
                verbose: false,
                config: None,
            },
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(false),
                crate::common::LockfileManager::new(true),
            ),
        );

        assert!(
            matches!(
                result,
                Err(crate::error::EngineError::LockfileUpdateRequired)
            ),
            "expected LockfileUpdateRequired, got: {result:?}"
        );
    }

    #[test]
    fn lint_locked_errors_when_jre_toolchain_tarball_hashes_missing() {
        // The detekt JRE comes from the managed Kotlin/Native toolchain. Under
        // --locked, a clean-machine install is only allowed when the toolchain
        // tarballs are pinned; a version-only lockfile entry must fail before
        // installing anything.
        let detekt_version = "99.0.6-locked-jre";
        let jar = super::detekt_jar_path(detekt_version).unwrap();
        std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
        let content = b"fake jar for locked jre test";
        std::fs::write(&jar, content).unwrap();
        let jar_hash = konvoy_util::hash::sha256_bytes(content);

        let kotlin_version = "0.0.0-locked-jre-test";
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("konvoy.toml"),
            format!(
                "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{kotlin_version}\"\ndetekt = \"{detekt_version}\"\n"
            ),
        )
        .unwrap();
        let lockfile = konvoy_config::lockfile::Lockfile {
            toolchain: Some(konvoy_config::lockfile::ToolchainLock {
                konanc_version: kotlin_version.to_owned(),
                konanc_tarball_sha256: None,
                jre_tarball_sha256: None,
                detekt_version: Some(detekt_version.to_owned()),
                detekt_jar_sha256: Some(jar_hash),
            }),
            ..Default::default()
        };
        lockfile.write_to(&root.join("konvoy.lock")).unwrap();

        let result = super::lint(
            root,
            &super::LintOptions {
                verbose: false,
                config: None,
            },
            crate::common::ArtifactResolver::new(
                &konvoy_util::net::NetworkClient::new(false),
                crate::common::LockfileManager::new(true),
            ),
        );

        let _ = std::fs::remove_file(&jar);
        let _ = std::fs::remove_dir(jar.parent().unwrap());

        assert!(
            matches!(
                result,
                Err(crate::error::EngineError::LockfileUpdateRequired)
            ),
            "expected LockfileUpdateRequired before JRE install, got: {result:?}"
        );
    }

    #[test]
    fn parse_detekt_combined_stdout_stderr() {
        // Simulate realistic combined stdout+stderr output from detekt.
        // Summary/timing lines (stdout) mixed with diagnostic findings (stderr).
        let combined = "\
detekt finished in 1234ms
src/main.kt:3:5: This expression contains a magic number. [MagicNumber]
src/util.kt:20:1: Method too long. [LongMethod]
Overall debt: 10min
src/app.kt:5:10: Empty function body. [EmptyFunctionBlock]
src/config.kt:15:1: Line is too long. [MaxLineLength]";
        let diags = parse_detekt_output(combined);
        assert_eq!(diags.len(), 4, "expected 4 findings, got {}", diags.len());
        assert_eq!(diags.get(0).map(|d| d.rule.as_str()), Some("MagicNumber"));
        assert_eq!(diags.get(1).map(|d| d.rule.as_str()), Some("LongMethod"));
        assert_eq!(
            diags.get(2).map(|d| d.rule.as_str()),
            Some("EmptyFunctionBlock")
        );
        assert_eq!(diags.get(3).map(|d| d.rule.as_str()), Some("MaxLineLength"));
    }

    #[test]
    fn parse_detekt_real_output() {
        // Real detekt 1.23.7 output copied verbatim.
        let output = "\
/tmp/project/src/main.kt:10:28: Empty catch block detected. If the exception can be safely ignored, name the exception according to one of the exemptions as per the configuration of this rule. [EmptyCatchBlock]
/tmp/project/src/main.kt:2:13: This expression contains a magic number. Consider defining it to a well named constant. [MagicNumber]";
        let diags = parse_detekt_output(output);
        assert_eq!(diags.len(), 2, "expected 2 findings, got {}", diags.len());

        assert_eq!(
            diags.first().map(|d| d.file.as_deref()),
            Some(Some("/tmp/project/src/main.kt"))
        );
        assert_eq!(diags.first().map(|d| d.line), Some(Some(10)));
        assert_eq!(
            diags.first().map(|d| d.rule.as_str()),
            Some("EmptyCatchBlock")
        );
        assert_eq!(
            diags.first().map(|d| d.message.as_str()),
            Some("Empty catch block detected. If the exception can be safely ignored, name the exception according to one of the exemptions as per the configuration of this rule.")
        );

        assert_eq!(
            diags.get(1).map(|d| d.file.as_deref()),
            Some(Some("/tmp/project/src/main.kt"))
        );
        assert_eq!(diags.get(1).map(|d| d.line), Some(Some(2)));
        assert_eq!(diags.get(1).map(|d| d.rule.as_str()), Some("MagicNumber"));
        assert_eq!(
            diags.get(1).map(|d| d.message.as_str()),
            Some("This expression contains a magic number. Consider defining it to a well named constant.")
        );
    }

    #[test]
    fn resolve_config_errors_on_missing_explicit_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let missing = std::path::Path::new("nonexistent.yml");
        let result = resolve_config(root, Some(missing));
        assert!(result.is_err(), "expected Err for missing explicit config");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("config file not found"), "error was: {err}");
        assert!(
            err.contains("nonexistent.yml"),
            "error should mention the path: {err}"
        );
    }

    #[test]
    fn resolve_config_returns_existing_explicit_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let cfg_path = root.join("my-detekt.yml");
        std::fs::write(&cfg_path, "# config").unwrap();
        let result = resolve_config(root, Some(std::path::Path::new("my-detekt.yml")));
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert_eq!(result.unwrap(), Some(cfg_path));
    }

    #[test]
    fn resolve_config_returns_absolute_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let cfg_path = tmp.path().join("absolute-detekt.yml");
        std::fs::write(&cfg_path, "# config").unwrap();
        let result = resolve_config(root, Some(&cfg_path));
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert_eq!(result.unwrap(), Some(cfg_path));
    }

    #[test]
    fn resolve_config_uses_default_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let default_cfg = root.join("detekt.yml");
        std::fs::write(&default_cfg, "# default config").unwrap();
        let result = resolve_config(root, None);
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert_eq!(result.unwrap(), Some(default_cfg));
    }

    #[test]
    fn resolve_config_returns_none_when_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let result = resolve_config(root, None);
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert_eq!(result.unwrap(), None);
    }
}
