use sha2::{Digest, Sha256};
use std::path::Path;

/// Compute the SHA-256 hex digest of a byte slice.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Compute the SHA-256 hex digest of a file's contents.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let data = std::fs::read(path)?;
    Ok(sha256_bytes(&data))
}
