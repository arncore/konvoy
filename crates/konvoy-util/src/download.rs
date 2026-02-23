//! Shared HTTP download with progress reporting and SHA-256 hashing.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::UtilError;

/// Convert `usize` to `u64`. Infallible on 32-bit and 64-bit platforms.
fn u64_from_usize(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Compute download percentage as a `u8` (0..=100).
fn pct_u8(downloaded: u64, total: u64) -> u8 {
    u8::try_from((downloaded * 100) / total).unwrap_or(100)
}

/// Download a URL to a file, showing progress on stderr and computing SHA-256.
///
/// Returns the hex-encoded SHA-256 hash of the downloaded content.
///
/// # Errors
/// Returns an error if the HTTP request fails, the file cannot be written,
/// or a read error occurs during streaming.
pub fn download_with_progress(
    url: &str,
    dest: &Path,
    label: &str,
    version: &str,
) -> Result<String, UtilError> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build(),
    );

    let response = agent.get(url).call().map_err(|e| UtilError::Download {
        message: e.to_string(),
    })?;

    let content_length: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    let mut body = response.into_body();
    let mut file = std::fs::File::create(dest).map_err(|source| UtilError::Io {
        path: dest.display().to_string(),
        source,
    })?;

    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_pct: u8 = 0;
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

        if let Some(total) = content_length {
            if total > 0 {
                let pct = pct_u8(downloaded, total);
                if pct != last_pct && pct.is_multiple_of(10) {
                    eprint!("\r    Downloading {label} {version}... {pct}%");
                    last_pct = pct;
                }
            }
        }
    }

    if content_length.is_some() {
        eprintln!("\r    Downloading {label} {version}... done   ");
    } else {
        let mb = downloaded / (1024 * 1024);
        eprintln!("    Downloaded {label} {version} ({mb} MB)");
    }

    Ok(format!("{:x}", hasher.finalize()))
}
