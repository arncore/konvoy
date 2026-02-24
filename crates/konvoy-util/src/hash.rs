//! Hashing utilities for deterministic cache key computation.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::UtilError;

/// Compute the SHA-256 hex digest of a byte slice.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Compute the SHA-256 hex digest of a file using streaming reads.
///
/// Uses a 64 KiB buffer to avoid loading the entire file into memory,
/// which matters for large files like detekt JARs (~50 MB).
///
/// # Errors
/// Returns an error if the file cannot be opened or read.
pub fn sha256_file(path: &Path) -> Result<String, UtilError> {
    let file = std::fs::File::open(path).map_err(|source| UtilError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf).map_err(|source| UtilError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if n == 0 {
            break;
        }
        let Some(chunk) = buf.get(..n) else {
            break; // unreachable: n is bounded by buf.len()
        };
        hasher.update(chunk);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Hash all files matching `pattern` inside `dir`, sorted by relative path for determinism.
///
/// The `pattern` is a glob expression (e.g. `"**/*.kt"`). Files are sorted by their
/// path relative to `dir` before hashing so the result is stable across runs.
///
/// # Errors
/// Returns an error if the glob pattern is invalid, `dir` cannot be read, or any
/// matched file cannot be read.
pub fn sha256_dir(dir: &Path, pattern: &str) -> Result<String, UtilError> {
    let full_pattern = dir.join(pattern);
    let full_pattern_str = full_pattern.display().to_string();

    let mut paths: Vec<_> = glob::glob(&full_pattern_str)
        .map_err(|e| UtilError::GlobPattern {
            pattern: full_pattern_str.clone(),
            message: e.to_string(),
        })?
        .filter_map(Result::ok)
        .filter(|p| p.is_file())
        .collect();

    paths.sort();

    let mut hasher = Sha256::new();
    for path in &paths {
        // Include the relative path in the hash so renames are detected.
        let relative = path.strip_prefix(dir).unwrap_or(path);
        hasher.update(relative.display().to_string().as_bytes());

        let data = std::fs::read(path).map_err(|source| UtilError::Io {
            path: path.display().to_string(),
            source,
        })?;
        hasher.update(&data);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Combine multiple string parts into a single composite SHA-256 hash.
///
/// Each part is hashed in order with a length prefix to prevent ambiguity.
pub fn sha256_multi(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        // Length-prefix each part to avoid collisions like ["ab","c"] vs ["a","bc"].
        let len_bytes = part.len().to_le_bytes();
        hasher.update(len_bytes);
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn sha256_bytes_deterministic() {
        let a = sha256_bytes(b"hello");
        let b = sha256_bytes(b"hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // 256 bits = 64 hex chars
    }

    #[test]
    fn sha256_bytes_different_input() {
        let a = sha256_bytes(b"hello");
        let b = sha256_bytes(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn sha256_bytes_empty() {
        let hash = sha256_bytes(b"");
        // Known SHA-256 of empty input
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_file_reads_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, b"file content").unwrap();

        let hash = sha256_file(&file).unwrap();
        let expected = sha256_bytes(b"file content");
        assert_eq!(hash, expected);
    }

    #[test]
    fn sha256_file_missing() {
        let result = sha256_file(Path::new("/nonexistent/path/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn sha256_dir_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.kt"), b"fun a()").unwrap();
        fs::write(dir.path().join("b.kt"), b"fun b()").unwrap();

        let hash1 = sha256_dir(dir.path(), "**/*.kt").unwrap();
        let hash2 = sha256_dir(dir.path(), "**/*.kt").unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn sha256_dir_order_independent_of_creation_order() {
        // Create files in one order
        let dir1 = tempfile::tempdir().unwrap();
        fs::write(dir1.path().join("b.kt"), b"fun b()").unwrap();
        fs::write(dir1.path().join("a.kt"), b"fun a()").unwrap();

        // Create files in reverse order
        let dir2 = tempfile::tempdir().unwrap();
        fs::write(dir2.path().join("a.kt"), b"fun a()").unwrap();
        fs::write(dir2.path().join("b.kt"), b"fun b()").unwrap();

        let hash1 = sha256_dir(dir1.path(), "**/*.kt").unwrap();
        let hash2 = sha256_dir(dir2.path(), "**/*.kt").unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn sha256_dir_different_content_different_hash() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.kt"), b"fun a()").unwrap();
        let hash1 = sha256_dir(dir.path(), "**/*.kt").unwrap();

        fs::write(dir.path().join("a.kt"), b"fun a_changed()").unwrap();
        let hash2 = sha256_dir(dir.path(), "**/*.kt").unwrap();

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn sha256_dir_empty_matches() {
        let dir = tempfile::tempdir().unwrap();
        // No .kt files
        fs::write(dir.path().join("readme.md"), b"hello").unwrap();

        let hash = sha256_dir(dir.path(), "**/*.kt").unwrap();
        // Should still produce a valid hash (of empty input)
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn sha256_multi_deterministic() {
        let a = sha256_multi(&["hello", "world"]);
        let b = sha256_multi(&["hello", "world"]);
        assert_eq!(a, b);
    }

    #[test]
    fn sha256_multi_order_matters() {
        let a = sha256_multi(&["hello", "world"]);
        let b = sha256_multi(&["world", "hello"]);
        assert_ne!(a, b);
    }

    #[test]
    fn sha256_multi_no_boundary_collision() {
        // ["ab", "c"] and ["a", "bc"] must produce different hashes
        let a = sha256_multi(&["ab", "c"]);
        let b = sha256_multi(&["a", "bc"]);
        assert_ne!(a, b);
    }

    #[test]
    fn sha256_multi_empty_parts() {
        let hash = sha256_multi(&[]);
        assert_eq!(hash.len(), 64);
    }
}
