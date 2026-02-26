//! Generic artifact download, SHA-256 verification, and atomic placement.

use std::path::{Path, PathBuf};

use crate::error::UtilError;

/// Result of ensuring an artifact is available locally.
#[derive(Debug, Clone)]
pub struct ArtifactResult {
    /// Path to the artifact on disk.
    pub path: PathBuf,
    /// Hex-encoded SHA-256 hash of the artifact.
    pub sha256: String,
    /// `true` if the artifact was downloaded this call, `false` if it already existed.
    pub freshly_downloaded: bool,
}

/// Validate that a version string is safe for filesystem paths and URLs.
///
/// Allows only `[a-zA-Z0-9._-]`. Must be non-empty.
///
/// # Errors
/// Returns `UtilError::InvalidVersion` if the string is empty or contains
/// characters outside the allowed set.
pub fn validate_version(version: &str) -> Result<(), UtilError> {
    if version.is_empty()
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(UtilError::InvalidVersion {
            version: version.to_owned(),
        });
    }
    Ok(())
}

/// Ensure a single-file artifact exists at `dest`, downloading from `url` if needed.
///
/// Follows the same atomic-download pattern as the detekt JAR management:
///
/// 1. If `dest` already exists, hash it and verify against `expected_sha256`
///    (if provided). Return immediately with `freshly_downloaded = false`.
/// 2. If missing, create parent directories, download to a temp file, verify
///    the hash, then atomically rename into place.
/// 3. Handle the race condition where another process downloads the artifact
///    concurrently: if the rename fails but `dest` now exists, verify the
///    placed file's hash.
/// 4. Clean up the temp file on all error paths.
///
/// # Errors
/// Returns an error if the download fails, the hash does not match, or an
/// I/O operation fails.
pub fn ensure_artifact(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    label: &str,
    version: &str,
) -> Result<ArtifactResult, UtilError> {
    // 1. If the file already exists, verify its hash and return early.
    if dest.exists() {
        let actual_hash = crate::hash::sha256_file(dest)?;
        if let Some(expected) = expected_sha256 {
            if actual_hash != expected {
                return Err(UtilError::ArtifactHashMismatch {
                    path: dest.display().to_string(),
                    expected: expected.to_owned(),
                    actual: actual_hash,
                });
            }
        }
        return Ok(ArtifactResult {
            path: dest.to_path_buf(),
            sha256: actual_hash,
            freshly_downloaded: false,
        });
    }

    // 2. Create parent directories.
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|source| UtilError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }

    // Build temp file path: ".tmp-{label}-{pid}.{ext}" in the same directory.
    let pid = std::process::id();
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("tmp");
    let tmp_name = format!(".tmp-{label}-{pid}.{ext}");
    let tmp_path = dest
        .parent()
        .map(|p| p.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(&tmp_name));

    // Download to temp file.
    let download_hash = crate::download::download_with_progress(url, &tmp_path, label, version)?;

    // Verify hash of downloaded file before placing it.
    if let Some(expected) = expected_sha256 {
        if download_hash != expected {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(UtilError::ArtifactHashMismatch {
                path: dest.display().to_string(),
                expected: expected.to_owned(),
                actual: download_hash,
            });
        }
    }

    // 3. Atomic rename into final location.
    match std::fs::rename(&tmp_path, dest) {
        Ok(()) => {}
        Err(_) if dest.exists() => {
            // Another process placed the file concurrently — verify its hash.
            let _ = std::fs::remove_file(&tmp_path);
            if let Some(expected) = expected_sha256 {
                let placed_hash = crate::hash::sha256_file(dest)?;
                if placed_hash != expected {
                    return Err(UtilError::ArtifactHashMismatch {
                        path: dest.display().to_string(),
                        expected: expected.to_owned(),
                        actual: placed_hash,
                    });
                }
            }
        }
        Err(source) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(UtilError::Io {
                path: dest.display().to_string(),
                source,
            });
        }
    }

    Ok(ArtifactResult {
        path: dest.to_path_buf(),
        sha256: download_hash,
        freshly_downloaded: true,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn validate_version_accepts_valid() {
        assert!(validate_version("1.23.7").is_ok());
        assert!(validate_version("2.0.0-RC1").is_ok());
        assert!(validate_version("1.0.0_beta").is_ok());
    }

    #[test]
    fn validate_version_rejects_empty() {
        let result = validate_version("");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid version"), "error was: {err}");
    }

    #[test]
    fn validate_version_rejects_path_traversal() {
        assert!(validate_version("../../etc").is_err());
        assert!(validate_version("../foo").is_err());
    }

    #[test]
    fn validate_version_rejects_special_chars() {
        assert!(validate_version("1.0; rm -rf /").is_err());
        assert!(validate_version("ver\0sion").is_err());
    }

    #[test]
    fn ensure_artifact_returns_existing_with_correct_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        let content = b"test artifact content";
        std::fs::write(&dest, content).unwrap();

        let expected_hash = crate::hash::sha256_bytes(content);

        let result = ensure_artifact(
            "http://unused.example.com/artifact.jar",
            &dest,
            Some(&expected_hash),
            "test",
            "1.0.0",
        )
        .unwrap();

        assert!(!result.freshly_downloaded);
        assert_eq!(result.sha256, expected_hash);
        assert_eq!(result.path, dest);
    }

    #[test]
    fn ensure_artifact_errors_on_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        std::fs::write(&dest, b"some content").unwrap();

        let bogus_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = ensure_artifact(
            "http://unused.example.com/artifact.jar",
            &dest,
            Some(bogus_hash),
            "test",
            "1.0.0",
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("artifact hash mismatch"), "error was: {err}");
    }

    #[test]
    fn ensure_artifact_returns_existing_without_expected_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        let content = b"no hash check content";
        std::fs::write(&dest, content).unwrap();

        let expected_hash = crate::hash::sha256_bytes(content);

        let result = ensure_artifact(
            "http://unused.example.com/artifact.jar",
            &dest,
            None,
            "test",
            "1.0.0",
        )
        .unwrap();

        assert!(!result.freshly_downloaded);
        assert_eq!(result.sha256, expected_hash);
    }

    #[test]
    fn ensure_artifact_errors_on_invalid_url() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("missing.jar");

        let result = ensure_artifact(
            "http://127.0.0.1:1/nonexistent",
            &dest,
            None,
            "test",
            "1.0.0",
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("download failed"), "error was: {err}");
    }
}
