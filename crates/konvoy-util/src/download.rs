//! Shared HTTP download with progress reporting and SHA-256 hashing.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::UtilError;
use crate::hash::finalize_hex;

/// Convert `usize` to `u64`. Infallible on 32-bit and 64-bit platforms.
fn u64_from_usize(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Create an HTTP agent with the given global timeout.
///
/// Uses a 30-second connect timeout for all requests.
pub fn http_agent(global_timeout_secs: u64) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_global(Some(std::time::Duration::from_secs(global_timeout_secs)))
            .build(),
    )
}

/// Stream a URL to a file, calling `on_progress` as bytes arrive and computing SHA-256.
///
/// `on_progress(downloaded, total)` is invoked after each chunk:
/// - `downloaded`: cumulative bytes written so far
/// - `total`: `Content-Length` from the response (or `None` if absent)
///
/// `total` is `None` until the response headers are parsed and the server
/// advertises a content length; from the first chunk onward it's either
/// always `Some(n)` for the same `n` or always `None`.
///
/// This function knows nothing about progress bars — callers in
/// `konvoy_util::progress` adapt it.
///
/// Returns the hex-encoded SHA-256 hash of the downloaded content.
///
/// # Errors
/// Returns an error if the HTTP request fails, the file cannot be written,
/// or a read error occurs during streaming.
pub fn stream_download<F>(url: &str, dest: &Path, mut on_progress: F) -> Result<String, UtilError>
where
    F: FnMut(u64, Option<u64>),
{
    let agent = http_agent(600);

    let response = agent.get(url).call().map_err(|e| UtilError::Download {
        message: e.to_string(),
    })?;

    let content_length: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .filter(|t: &u64| *t > 0);

    let mut body = response.into_body();
    let mut file = std::fs::File::create(dest).map_err(|source| UtilError::Io {
        path: dest.display().to_string(),
        source,
    })?;

    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = std::io::Read::read(&mut body.as_reader(), &mut buf).map_err(|e| {
            UtilError::Download {
                message: e.to_string(),
            }
        })?;
        if n == 0 {
            break;
        }

        let Some(chunk) = buf.get(..n) else {
            break; // unreachable: n is bounded by buf.len()
        };
        std::io::Write::write_all(&mut file, chunk).map_err(|source| UtilError::Io {
            path: dest.display().to_string(),
            source,
        })?;
        hasher.update(chunk);

        downloaded = downloaded.saturating_add(u64_from_usize(n));
        on_progress(downloaded, content_length);
    }

    Ok(finalize_hex(hasher))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{stream_download, u64_from_usize};

    /// No-op progress callback used by tests that don't care about events.
    fn ignore_progress(_: u64, _: Option<u64>) {}

    #[test]
    fn u64_from_usize_roundtrips() {
        assert_eq!(u64_from_usize(0), 0);
        assert_eq!(u64_from_usize(1024), 1024);
    }

    #[test]
    fn u64_from_usize_max_value() {
        // usize::MAX should convert to u64 on 64-bit, or saturate to u64::MAX on 128-bit+.
        let result = u64_from_usize(usize::MAX);
        assert!(result > 0);
    }

    #[test]
    fn download_invalid_url_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.bin");
        let result = stream_download("http://127.0.0.1:1/nonexistent", &dest, ignore_progress);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("download failed"), "error was: {err}");
    }

    #[test]
    fn download_malformed_url_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.bin");
        let result = stream_download("not-a-valid-url", &dest, ignore_progress);
        assert!(result.is_err());
    }

    #[test]
    fn download_unwritable_dest_returns_error() {
        let result = stream_download(
            "http://127.0.0.1:1/file.bin",
            std::path::Path::new("/nonexistent_root/subdir/out.bin"),
            ignore_progress,
        );
        assert!(result.is_err());
    }
}
