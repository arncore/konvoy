//! Generic artifact download, SHA-256 verification, and atomic placement.

use std::path::{Path, PathBuf};

use crate::error::UtilError;

/// Result of resolving an artifact locally — either a cache-hit reported by
/// [`check_cached`] or a fresh fetch from [`download_artifact`].
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

/// Validate that a Maven group or artifact identifier is safe for filesystem paths.
///
/// Allows only `[a-zA-Z0-9._-]`. Must be non-empty and must not contain `..`
/// path-traversal sequences. This protects functions that build cache paths
/// from identifier components (e.g. `~/.konvoy/cache/pom/<group>/<artifact>-<version>.pom`).
///
/// # Errors
/// Returns `UtilError::InvalidVersion` (re-used: the underlying constraint is
/// identical) if the string is empty, contains disallowed characters, or
/// contains a `..` sequence.
pub fn validate_identifier(identifier: &str) -> Result<(), UtilError> {
    // `..` would let a malicious coordinate escape the cache dir even though
    // every individual character is in the allowed set (`.` and `.` are both ok).
    if identifier.is_empty()
        || identifier.contains("..")
        || !identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(UtilError::InvalidVersion {
            version: identifier.to_owned(),
        });
    }
    Ok(())
}

/// Check whether `dest` is already on disk with the expected SHA-256.
///
/// Returns:
/// - `Ok(Some(ArtifactResult))` if the file exists and (when `expected_sha256`
///   is `Some`) its hash matches.
/// - `Ok(None)` if the file does not exist — caller should download.
/// - `Err(UtilError::ArtifactHashMismatch)` if the file exists but its hash
///   does not match `expected_sha256` (this is a hard error, not a "go
///   re-download" signal: a hash mismatch on a cached artifact usually
///   means the lockfile is stale or the cache was tampered with).
/// - Other `UtilError` variants for I/O failures reading the file.
///
/// This function does no UI work. Pair it with [`download_artifact`] when
/// the result is `None`.
///
/// # Errors
/// See above.
pub fn check_cached(
    dest: &Path,
    expected_sha256: Option<&str>,
) -> Result<Option<ArtifactResult>, UtilError> {
    if !dest.exists() {
        return Ok(None);
    }
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
    Ok(Some(ArtifactResult {
        path: dest.to_path_buf(),
        sha256: actual_hash,
        freshly_downloaded: false,
    }))
}

/// Download `url` to `dest`, atomically placing the file once the hash is
/// verified.
///
/// Steps:
/// 1. Create parent directories.
/// 2. Download to a `.tmp-{label}-{pid}` sibling.
/// 3. Verify the hash of the downloaded bytes.
/// 4. Atomically rename into place. If another process already placed a
///    file at `dest` concurrently (TOCTOU with `check_cached`), verify the
///    placed file's hash and report it as the result.
/// 5. Clean up the temp file on all error paths.
///
/// Callers should normally check [`check_cached`] first and only invoke
/// this on a cache miss. `on_progress(downloaded, total)` is invoked once
/// per chunk; the UI layer (`konvoy_util::progress`) wires this to a bar.
///
/// # Errors
/// Returns an error if the download fails, the hash does not match, or an
/// I/O operation fails.
pub fn download_artifact<F>(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    label: &str,
    on_progress: F,
) -> Result<ArtifactResult, UtilError>
where
    F: FnMut(u64, Option<u64>),
{
    // Create parent directories.
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
    let download_hash = crate::download::stream_download(url, &tmp_path, on_progress)?;

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

    // Atomic rename into final location.
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
    fn validate_identifier_accepts_typical_maven_ids() {
        assert!(validate_identifier("org.jetbrains.kotlinx").is_ok());
        assert!(validate_identifier("kotlinx-coroutines-core").is_ok());
        assert!(validate_identifier("kotlin_stdlib").is_ok());
        assert!(validate_identifier("a").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_path_traversal() {
        assert!(validate_identifier("..").is_err());
        assert!(validate_identifier("..foo").is_err());
        assert!(validate_identifier("foo..bar").is_err());
        assert!(validate_identifier("../etc").is_err());
    }

    #[test]
    fn validate_identifier_rejects_empty_and_special_chars() {
        assert!(validate_identifier("").is_err());
        assert!(validate_identifier("foo/bar").is_err());
        assert!(validate_identifier("foo bar").is_err());
        assert!(validate_identifier("foo\0bar").is_err());
        assert!(validate_identifier("foo:bar").is_err());
    }

    #[test]
    fn check_cached_returns_some_with_correct_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        let content = b"test artifact content";
        std::fs::write(&dest, content).unwrap();

        let expected_hash = crate::hash::sha256_bytes(content);

        let result = check_cached(&dest, Some(&expected_hash)).unwrap().unwrap();

        assert!(!result.freshly_downloaded);
        assert_eq!(result.sha256, expected_hash);
        assert_eq!(result.path, dest);
    }

    #[test]
    fn check_cached_returns_none_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("missing.jar");
        let result = check_cached(&dest, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn check_cached_errors_on_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        std::fs::write(&dest, b"some content").unwrap();

        let bogus_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = check_cached(&dest, Some(bogus_hash));

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("hash mismatch"), "error was: {err}");
        // Error must explain possible causes so the user understands what happened.
        assert!(
            err.contains("corrupted on disk"),
            "error should mention disk corruption: {err}"
        );
        assert!(
            err.contains("tampered with"),
            "error should mention tampering: {err}"
        );
        // Error must tell the user what to do.
        assert!(
            err.contains("inspect or delete"),
            "error should tell the user to inspect/delete the file: {err}"
        );
    }

    #[test]
    fn check_cached_returns_some_without_expected_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("artifact.jar");
        let content = b"no hash check content";
        std::fs::write(&dest, content).unwrap();

        let expected_hash = crate::hash::sha256_bytes(content);

        let result = check_cached(&dest, None).unwrap().unwrap();

        assert!(!result.freshly_downloaded);
        assert_eq!(result.sha256, expected_hash);
    }

    #[test]
    fn download_artifact_errors_on_invalid_url() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("missing.jar");

        let result = download_artifact(
            "http://127.0.0.1:1/nonexistent",
            &dest,
            None,
            "test",
            |_, _| {},
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("download failed"), "error was: {err}");
    }
}
