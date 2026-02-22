//! Compiler detection and version parsing.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::KonancError;

/// Information about a detected `konanc` installation.
#[derive(Debug, Clone)]
pub struct KonancInfo {
    /// Absolute path to the `konanc` binary.
    pub path: PathBuf,
    /// Parsed semantic version (e.g. "2.1.0").
    pub version: String,
    /// SHA-256 hex digest of the `konanc` binary, used for cache keying.
    pub fingerprint: String,
}

/// Locate `konanc` and determine its version and fingerprint.
///
/// Resolution order:
/// 1. `KONANC_HOME` environment variable (`$KONANC_HOME/bin/konanc`)
/// 2. `PATH` lookup via `which`
///
/// # Errors
/// Returns an error if `konanc` is not found, is not executable, returns an
/// unparseable version string, or cannot be fingerprinted.
pub fn detect_konanc() -> Result<KonancInfo, KonancError> {
    let path = resolve_konanc_path()?;
    check_executable(&path)?;
    let version = query_version(&path)?;
    let fingerprint = compute_fingerprint(&path)?;

    Ok(KonancInfo {
        path,
        version,
        fingerprint,
    })
}

/// Parse a semver version from raw `konanc -version` output.
///
/// Handles formats like:
/// - `info: kotlinc-native 2.1.0 (JRE 17.0.2+8)`
/// - `kotlinc-native 2.1.0`
/// - `2.1.0`
pub fn parse_version(raw: &str) -> Option<String> {
    // Look for a semver-like pattern: digits.digits.digits (optional -suffix)
    for token in raw.split_whitespace() {
        let trimmed = token.trim_start_matches('v');
        if is_semver_like(trimmed) {
            return Some(trimmed.to_owned());
        }
    }
    None
}

fn is_semver_like(s: &str) -> bool {
    let mut parts = s.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch_part) = parts.next() else {
        return false;
    };
    // No more than 3 dot-separated components for basic semver
    if parts.next().is_some() {
        return false;
    }

    // patch_part may contain a pre-release suffix like "0-beta1"
    let patch = patch_part.split('-').next().unwrap_or(patch_part);

    major.chars().all(|c| c.is_ascii_digit())
        && minor.chars().all(|c| c.is_ascii_digit())
        && patch.chars().all(|c| c.is_ascii_digit())
}

fn resolve_konanc_path() -> Result<PathBuf, KonancError> {
    if let Ok(home) = std::env::var("KONANC_HOME") {
        let p = PathBuf::from(home).join("bin").join("konanc");
        if p.exists() {
            return Ok(p);
        }
        return Err(KonancError::NotFound);
    }

    which_konanc().ok_or(KonancError::NotFound)
}

fn which_konanc() -> Option<PathBuf> {
    let output = Command::new("which").arg("konanc").output().ok()?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        let trimmed = path_str.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(PathBuf::from(trimmed))
    } else {
        None
    }
}

fn check_executable(path: &Path) -> Result<(), KonancError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path).map_err(|_| KonancError::NotExecutable {
            path: path.to_path_buf(),
        })?;
        let permissions = metadata.permissions();
        // Check user/group/other execute bits
        if permissions.mode() & 0o111 == 0 {
            return Err(KonancError::NotExecutable {
                path: path.to_path_buf(),
            });
        }
    }
    Ok(())
}

fn query_version(path: &PathBuf) -> Result<String, KonancError> {
    let output = Command::new(path)
        .arg("-version")
        .output()
        .map_err(|source| KonancError::Exec { source })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // konanc may print version to stdout or stderr depending on the version.
    let raw = if stdout.trim().is_empty() {
        stderr.trim().to_owned()
    } else {
        stdout.trim().to_owned()
    };

    parse_version(&raw).ok_or_else(|| KonancError::VersionParse {
        output: raw.clone(),
    })
}

fn compute_fingerprint(path: &Path) -> Result<String, KonancError> {
    konvoy_util::hash::sha256_file(path).map_err(|source| KonancError::Fingerprint {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_kotlinc_native_format() {
        let raw = "info: kotlinc-native 2.1.0 (JRE 17.0.2+8)";
        assert_eq!(parse_version(raw), Some("2.1.0".to_owned()));
    }

    #[test]
    fn parse_version_simple_format() {
        assert_eq!(
            parse_version("kotlinc-native 2.1.0"),
            Some("2.1.0".to_owned())
        );
    }

    #[test]
    fn parse_version_bare() {
        assert_eq!(parse_version("2.1.0"), Some("2.1.0".to_owned()));
    }

    #[test]
    fn parse_version_with_v_prefix() {
        assert_eq!(parse_version("v2.1.0"), Some("2.1.0".to_owned()));
    }

    #[test]
    fn parse_version_with_prerelease() {
        assert_eq!(
            parse_version("kotlinc-native 2.1.0-beta1"),
            Some("2.1.0-beta1".to_owned())
        );
    }

    #[test]
    fn parse_version_no_version() {
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn parse_version_empty() {
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn is_semver_like_valid() {
        assert!(is_semver_like("2.1.0"));
        assert!(is_semver_like("0.0.1"));
        assert!(is_semver_like("10.20.30"));
    }

    #[test]
    fn is_semver_like_with_prerelease() {
        assert!(is_semver_like("2.1.0-beta1"));
    }

    #[test]
    fn is_semver_like_invalid() {
        assert!(!is_semver_like("2.1"));
        assert!(!is_semver_like("2"));
        assert!(!is_semver_like("abc"));
        assert!(!is_semver_like("2.1.0.4"));
    }

    #[test]
    fn error_messages_are_actionable() {
        let not_found = KonancError::NotFound;
        let msg = not_found.to_string();
        assert!(msg.contains("install"));
        assert!(msg.contains("PATH"));

        let not_exec = KonancError::NotExecutable {
            path: PathBuf::from("/usr/bin/konanc"),
        };
        let msg = not_exec.to_string();
        assert!(msg.contains("not executable"));
        assert!(msg.contains("permissions"));
    }
}
