//! Shared HTTP download with progress reporting and SHA-256 hashing.

use std::path::Path;

use indicatif::ProgressBar;
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

/// Download a URL to a file, updating `progress` as bytes arrive and computing SHA-256.
///
/// The caller owns the `ProgressBar` — set its prefix/style before calling so
/// it can host a single download or attach to a [`indicatif::MultiProgress`].
/// If the response has a `Content-Length`, the bar's length is set; otherwise
/// the bar is switched to spinner mode.
///
/// Returns the hex-encoded SHA-256 hash of the downloaded content.
///
/// # Errors
/// Returns an error if the HTTP request fails, the file cannot be written,
/// or a read error occurs during streaming.
pub fn download_with_progress(
    url: &str,
    dest: &Path,
    progress: &ProgressBar,
) -> Result<String, UtilError> {
    let agent = http_agent(600);

    let response = agent.get(url).call().map_err(|e| UtilError::Download {
        message: e.to_string(),
    })?;

    let content_length: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    match content_length {
        Some(total) if total > 0 => {
            progress.set_length(total);
            // Force the bar to redraw with the new length before the download
            // races to completion. `set_length` alone does not trigger a redraw
            // on `MultiProgress` children at indicatif's 20Hz throttle, so a
            // sub-50ms download can finish before the length ever renders.
            progress.set_position(0);
        }
        _ => crate::progress::switch_to_spinner(progress),
    }

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
        progress.set_position(downloaded);
    }

    progress.finish();
    Ok(finalize_hex(hasher))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::u64_from_usize;
    use crate::progress::hidden_bar;

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
        let result =
            super::download_with_progress("http://127.0.0.1:1/nonexistent", &dest, &hidden_bar());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("download failed"), "error was: {err}");
    }

    #[test]
    fn download_malformed_url_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.bin");
        let result = super::download_with_progress("not-a-valid-url", &dest, &hidden_bar());
        assert!(result.is_err());
    }

    #[test]
    fn download_unwritable_dest_returns_error() {
        let result = super::download_with_progress(
            "http://127.0.0.1:1/file.bin",
            std::path::Path::new("/nonexistent_root/subdir/out.bin"),
            &hidden_bar(),
        );
        assert!(result.is_err());
    }
}
