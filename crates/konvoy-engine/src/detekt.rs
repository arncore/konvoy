//! Detekt tool management: download, invocation, and output parsing.
//!
//! Downloads `detekt-cli` fat JARs from GitHub releases and runs them
//! against Kotlin source files using the JRE bundled with managed toolchains.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::EngineError;

/// Map a `UtilError::Download` to `EngineError::DetektDownload`.
fn map_download_err(version: &str, e: konvoy_util::error::UtilError) -> EngineError {
    match e {
        konvoy_util::error::UtilError::Download { message } => EngineError::DetektDownload {
            version: version.to_owned(),
            message,
        },
        other => EngineError::Util(other),
    }
}

/// Options for the `lint` command.
#[derive(Debug, Clone)]
pub struct LintOptions {
    /// Whether to show raw detekt output.
    pub verbose: bool,
    /// Optional path to a custom detekt configuration file.
    pub config: Option<PathBuf>,
    /// Require the lockfile to be up-to-date; error on any mismatch or missing hash.
    pub locked: bool,
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

/// Return the root directory for managed tools: `~/.konvoy/tools/`.
fn tools_dir() -> Result<PathBuf, EngineError> {
    Ok(konvoy_util::fs::konvoy_home()?.join("tools"))
}

/// Return the directory for a specific detekt version.
fn detekt_dir(version: &str) -> Result<PathBuf, EngineError> {
    Ok(tools_dir()?.join("detekt").join(version))
}

/// Return the path to the detekt-cli JAR for a specific version.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn detekt_jar_path(version: &str) -> Result<PathBuf, EngineError> {
    Ok(detekt_dir(version)?.join(format!("detekt-cli-{version}-all.jar")))
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
    let jar = detekt_jar_path(version)?;
    Ok(jar.exists())
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
) -> Result<(PathBuf, String), EngineError> {
    validate_version(version)?;
    let jar = detekt_jar_path(version)?;

    if jar.exists() {
        // Verify hash of existing JAR.
        let actual_hash = konvoy_util::hash::sha256_file(&jar)?;
        if let Some(expected) = expected_sha256 {
            if actual_hash != expected {
                return Err(EngineError::DetektHashMismatch {
                    version: version.to_owned(),
                    expected: expected.to_owned(),
                    actual: actual_hash,
                });
            }
        }
        return Ok((jar, actual_hash));
    }

    let dir = detekt_dir(version)?;
    std::fs::create_dir_all(&dir).map_err(|source| EngineError::Io {
        path: dir.display().to_string(),
        source,
    })?;

    let url = detekt_download_url(version);

    // Download to a temp file, then rename atomically.
    let pid = std::process::id();
    let tmp_path = dir.join(format!(".tmp-detekt-{pid}.jar"));

    let download_hash =
        konvoy_util::download::download_with_progress(&url, &tmp_path, "detekt", version)
            .map_err(|e| map_download_err(version, e))?;

    // Verify hash before placing the file.
    if let Some(expected) = expected_sha256 {
        if download_hash != expected {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(EngineError::DetektHashMismatch {
                version: version.to_owned(),
                expected: expected.to_owned(),
                actual: download_hash,
            });
        }
    }

    // Atomic rename.
    match std::fs::rename(&tmp_path, &jar) {
        Ok(()) => {}
        Err(_) if jar.exists() => {
            // Another process downloaded it concurrently — verify its hash.
            let _ = std::fs::remove_file(&tmp_path);
            if let Some(expected) = expected_sha256 {
                let placed_hash = konvoy_util::hash::sha256_file(&jar)?;
                if placed_hash != expected {
                    return Err(EngineError::DetektHashMismatch {
                        version: version.to_owned(),
                        expected: expected.to_owned(),
                        actual: placed_hash,
                    });
                }
            }
        }
        Err(source) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(EngineError::Io {
                path: jar.display().to_string(),
                source,
            });
        }
    }

    Ok((jar, download_hash))
}

/// Validate that a version string is safe for use in filesystem paths.
/// Only allows alphanumeric characters, dots, hyphens, and underscores.
/// Notably excludes `+` (semver build metadata) since detekt releases don't
/// use it, and `+` can be problematic in URLs and filesystem paths.
fn validate_version(version: &str) -> Result<(), EngineError> {
    if version.is_empty()
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(EngineError::DetektDownload {
            version: version.to_owned(),
            message: format!(
                "invalid detekt version \"{version}\" — only alphanumeric characters, dots, hyphens, and underscores are allowed"
            ),
        });
    }
    Ok(())
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
    updated.write_to(lockfile_path).map_err(|e| {
        EngineError::Lockfile(format!("cannot write {}: {e}", lockfile_path.display()))
    })
}

/// Resolve the JRE `java` binary from the managed Kotlin toolchain.
///
/// Auto-installs the toolchain if it is not yet present.
fn resolve_java_bin(kotlin_version: &str) -> Result<(PathBuf, PathBuf), EngineError> {
    if !konvoy_konanc::toolchain::is_installed(kotlin_version)? {
        eprintln!("    Installing Kotlin/Native {kotlin_version} (for JRE)...");
        konvoy_konanc::toolchain::install(kotlin_version)?;
    }

    let jre_home = konvoy_konanc::toolchain::jre_home_path(kotlin_version)?;

    let java_bin = jre_home.join("bin").join("java");
    if !java_bin.exists() {
        return Err(EngineError::DetektNoJre);
    }

    Ok((java_bin, jre_home))
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
    java_bin: &Path,
    jre_home: &Path,
    jar_path: &Path,
    src_dir: &Path,
    config_path: Option<&Path>,
    detekt_version: &str,
    verbose: bool,
) -> Result<LintResult, EngineError> {
    let mut args = vec![
        "-jar".to_owned(),
        jar_path.display().to_string(),
        "--input".to_owned(),
        src_dir.display().to_string(),
    ];

    if let Some(cfg) = config_path {
        args.push("--config".to_owned());
        args.push(cfg.display().to_string());
        args.push("--build-upon-default-config".to_owned());
    }

    eprintln!("    Linting with detekt {detekt_version}...");

    let output = Command::new(java_bin)
        .args(&args)
        .env("JAVA_HOME", jre_home)
        .output()
        .map_err(|e| EngineError::DetektExec {
            message: e.to_string(),
        })?;

    let raw_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let raw_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let raw_output = format!("{raw_stdout}{raw_stderr}");

    if verbose {
        if !raw_stdout.is_empty() {
            eprintln!("{raw_stdout}");
        }
        if !raw_stderr.is_empty() {
            eprintln!("{raw_stderr}");
        }
    }

    let diagnostics = parse_detekt_output(&raw_output);
    let finding_count = diagnostics.len();
    let success = output.status.success();

    Ok(LintResult {
        success,
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
pub fn lint(root: &Path, options: &LintOptions) -> Result<LintResult, EngineError> {
    let manifest = konvoy_config::Manifest::from_path(&root.join("konvoy.toml"))?;

    let detekt_version = manifest
        .toolchain
        .detekt
        .as_deref()
        .ok_or(EngineError::LintNotConfigured)?;

    // Read lockfile and resolve expected hash.
    let lockfile_path = root.join("konvoy.lock");
    let lockfile = konvoy_config::lockfile::Lockfile::from_path(&lockfile_path)
        .map_err(|e| EngineError::Lockfile(e.to_string()))?;
    let expected_hash = resolve_lockfile_hash(&lockfile, detekt_version);

    // In --locked mode, require pinned hash and pre-downloaded JAR.
    if options.locked {
        if expected_hash.is_none() {
            return Err(EngineError::LockfileUpdateRequired);
        }
        if !is_installed(detekt_version)? {
            return Err(EngineError::DetektDownload {
                version: detekt_version.to_owned(),
                message: "detekt JAR not downloaded and --locked prevents downloads".to_owned(),
            });
        }
    }

    // Ensure detekt jar is available and hash-verified.
    let (jar_path, actual_hash) = ensure_detekt(detekt_version, expected_hash)?;

    // Persist hash to lockfile if not already stored.
    if expected_hash.is_none() {
        persist_detekt_hash(
            &lockfile_path,
            lockfile,
            &manifest.toolchain.kotlin,
            detekt_version,
            actual_hash,
        )?;
    }

    // Resolve JRE.
    let (java_bin, jre_home) = resolve_java_bin(&manifest.toolchain.kotlin)?;

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
        &java_bin,
        &jre_home,
        &jar_path,
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

/// Parse a single line of detekt output into a diagnostic.
///
/// Handles two formats:
/// - Real detekt 1.23.x: `path/file.kt:line:col: message text [RuleName]`
/// - Legacy format:      `path/file.kt:line:col: RuleName - message [detekt.RuleSet]`
fn parse_detekt_line(line: &str) -> Option<DetektDiagnostic> {
    // Find the pattern: ":<digits>:" which indicates file:line:
    // We need at least one colon after a file path.

    // Strategy: find the first occurrence of ":<digits>:" pattern.
    let chars = line.char_indices();
    let mut file_end = None;
    let mut line_num = None;
    let mut rest_start = 0;

    for (i, ch) in chars {
        if ch == ':' {
            // Check if followed by digits then colon
            let remaining = line.get(i + 1..)?;
            if let Some(end) = remaining.find(':') {
                let potential_num = remaining.get(..end)?;
                if let Ok(num) = potential_num.parse::<u32>() {
                    if file_end.is_none() {
                        file_end = Some(i);
                        line_num = Some(num);
                        // Skip past the line number and colon.
                        // Check if there's another number (column) after.
                        let after_line = remaining.get(end + 1..)?;
                        if let Some(col_end) = after_line.find(':') {
                            let potential_col = after_line.get(..col_end)?;
                            if potential_col.parse::<u32>().is_ok() {
                                // Skip column number too.
                                rest_start = i + 1 + end + 1 + col_end + 1;
                            } else {
                                rest_start = i + 1 + end + 1;
                            }
                        } else {
                            rest_start = i + 1 + end + 1;
                        }
                        break;
                    }
                }
            }
        }
    }

    let file = file_end.and_then(|end| {
        let f = line.get(..end)?.trim();
        if f.is_empty() {
            None
        } else {
            Some(f.to_owned())
        }
    });

    let rest = line.get(rest_start..)?.trim();
    if rest.is_empty() {
        return None;
    }

    // Try legacy format first: "RuleName - message [detekt.RuleSet]"
    // The legacy format has " - " separating a single-word rule from the message,
    // and the bracket contains a dotted category like "detekt.style".
    if let Some(dash_pos) = rest.find(" - ") {
        let candidate_rule = rest.get(..dash_pos)?.trim();
        // Legacy rule names are single PascalCase identifiers (no spaces, no dots).
        let is_legacy_rule = !candidate_rule.is_empty()
            && !candidate_rule.contains(' ')
            && !candidate_rule.contains('.');
        if is_legacy_rule {
            let msg = rest.get(dash_pos + 3..)?;
            // Strip trailing [detekt.RuleSet] if present.
            let msg = if let Some(bracket_pos) = msg.rfind('[') {
                msg.get(..bracket_pos)?.trim()
            } else {
                msg.trim()
            };
            return Some(DetektDiagnostic {
                rule: candidate_rule.to_owned(),
                message: msg.to_owned(),
                file,
                line: line_num,
            });
        }
    }

    // Real detekt format: "message text [RuleName]"
    // The rule name is inside square brackets at the end of the line.
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

    Some(DetektDiagnostic {
        rule: rule.to_owned(),
        message: message.to_owned(),
        file,
        line: line_num,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{
        detekt_download_url, detekt_jar_path, parse_detekt_output, resolve_config, validate_version,
    };

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
    fn validate_version_accepts_valid() {
        assert!(validate_version("1.23.7").is_ok());
        assert!(validate_version("2.0.0-RC1").is_ok());
        assert!(validate_version("1.0.0_beta").is_ok());
    }

    #[test]
    fn validate_version_rejects_path_traversal() {
        assert!(validate_version("../../etc").is_err());
        assert!(validate_version("../foo").is_err());
        assert!(validate_version("1.0/../../etc").is_err());
    }

    #[test]
    fn validate_version_rejects_empty() {
        assert!(validate_version("").is_err());
    }

    #[test]
    fn validate_version_rejects_special_chars() {
        assert!(validate_version("1.0; rm -rf /").is_err());
        assert!(validate_version("ver\0sion").is_err());
    }

    #[test]
    fn ensure_detekt_rejects_invalid_version() {
        let result = super::ensure_detekt("../../etc", None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid detekt version"), "error was: {err}");
    }

    #[test]
    fn ensure_detekt_rejects_empty_version() {
        let result = super::ensure_detekt("", None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid detekt version"), "error was: {err}");
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
        let result = super::ensure_detekt(version, Some(&real_hash));
        // Clean up.
        let _ = std::fs::remove_file(&jar);
        let _ = std::fs::remove_dir(jar.parent().unwrap());

        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        let (path, hash) = result.unwrap();
        assert_eq!(hash, real_hash);
        assert!(path.display().to_string().contains(version));
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
