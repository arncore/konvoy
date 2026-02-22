//! Compiler detection and version parsing.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::KonancError;
use crate::toolchain;

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

/// Result of resolving a managed konanc toolchain.
#[derive(Debug, Clone)]
pub struct ResolvedKonanc {
    /// Compiler information (path, version, fingerprint).
    pub info: KonancInfo,
    /// SHA-256 of the downloaded Kotlin/Native tarball, if this was a managed install.
    pub konanc_tarball_sha256: Option<String>,
    /// SHA-256 of the downloaded JRE tarball, if this was a managed install.
    pub jre_tarball_sha256: Option<String>,
    /// JAVA_HOME path for the bundled JRE.
    pub jre_home: Option<PathBuf>,
}

/// Resolve a managed `konanc` installation for the given version.
///
/// If the requested version is not installed, downloads and installs it
/// from JetBrains GitHub releases. After installation, verifies the version
/// matches and computes a fingerprint for cache keying.
///
/// # Errors
/// Returns an error if the toolchain cannot be installed, the version
/// doesn't match, or the binary cannot be fingerprinted.
pub fn resolve_konanc(version: &str) -> Result<ResolvedKonanc, KonancError> {
    let installed = toolchain::is_installed(version)?;

    let (konanc_tarball_sha256, jre_tarball_sha256) = if !installed {
        eprintln!("    Installing Kotlin/Native {version}...");
        let result = toolchain::install(version)?;
        let konanc_sha = if result.konanc_tarball_sha256.is_empty() {
            None
        } else {
            Some(result.konanc_tarball_sha256)
        };
        let jre_sha = if result.jre_tarball_sha256.is_empty() {
            None
        } else {
            Some(result.jre_tarball_sha256)
        };
        (konanc_sha, jre_sha)
    } else {
        (None, None)
    };

    let path = toolchain::managed_konanc_path(version)?;
    check_executable(&path)?;

    // Resolve bundled JRE for version queries and compilation.
    let jre_home = toolchain::jre_home_path(version).ok();

    let actual_version = query_version(&path, jre_home.as_deref())?;

    // Verify the installed version matches what was requested.
    if actual_version != version {
        return Err(KonancError::VersionMismatch {
            expected: version.to_owned(),
            actual: actual_version,
        });
    }

    let fingerprint = compute_fingerprint(&path)?;

    Ok(ResolvedKonanc {
        info: KonancInfo {
            path,
            version: actual_version,
            fingerprint,
        },
        konanc_tarball_sha256,
        jre_tarball_sha256,
        jre_home,
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

fn query_version(path: &PathBuf, java_home: Option<&Path>) -> Result<String, KonancError> {
    let mut cmd = Command::new(path);
    cmd.arg("-version");
    if let Some(jh) = java_home {
        cmd.env("JAVA_HOME", jh);
    }
    let output = cmd
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
}
