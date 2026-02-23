//! Filesystem utilities for Konvoy.

use std::path::{Path, PathBuf};

use crate::error::UtilError;

/// Create a directory and all parent directories if they do not exist.
///
/// # Errors
/// Returns an error if the directory cannot be created.
pub fn ensure_dir(path: &Path) -> Result<(), UtilError> {
    std::fs::create_dir_all(path).map_err(|source| UtilError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Copy `src` to `dest`, preferring a hard link for speed.
///
/// Falls back to a regular copy if hard linking fails (e.g. cross-device).
///
/// # Errors
/// Returns an error if both hard linking and copying fail.
pub fn materialize(src: &Path, dest: &Path) -> Result<(), UtilError> {
    // Ensure the parent directory exists.
    if let Some(parent) = dest.parent() {
        ensure_dir(parent)?;
    }

    // Remove existing destination if present, so hard_link doesn't fail.
    if dest.exists() {
        std::fs::remove_file(dest).map_err(|source| UtilError::Io {
            path: dest.display().to_string(),
            source,
        })?;
    }

    // Try hard link first, fall back to copy.
    if std::fs::hard_link(src, dest).is_err() {
        std::fs::copy(src, dest).map_err(|source| UtilError::Io {
            path: dest.display().to_string(),
            source,
        })?;
    }

    Ok(())
}

/// Remove a directory and all its contents. No error if the directory is absent.
///
/// # Errors
/// Returns an error if the directory exists but cannot be removed.
pub fn remove_dir_all_if_exists(path: &Path) -> Result<(), UtilError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(UtilError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Return the Konvoy home directory (`~/.konvoy`).
///
/// Resolves via `HOME` (Unix) or `USERPROFILE` (Windows).
///
/// # Errors
/// Returns an error if neither environment variable is set.
pub fn konvoy_home() -> Result<PathBuf, UtilError> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| UtilError::Io {
            path: "~/.konvoy".to_owned(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot determine home directory — set the HOME environment variable",
            ),
        })?;
    Ok(home.join(".konvoy"))
}

/// Collect all files with the given `extension` under `dir`, recursively, sorted by path.
///
/// # Errors
/// Returns an error if `dir` cannot be read.
pub fn collect_files(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, UtilError> {
    let mut files = Vec::new();
    collect_files_recursive(dir, extension, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_recursive(
    dir: &Path,
    extension: &str,
    out: &mut Vec<PathBuf>,
) -> Result<(), UtilError> {
    let entries = std::fs::read_dir(dir).map_err(|source| UtilError::Io {
        path: dir.display().to_string(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| UtilError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();

        if path.is_dir() {
            collect_files_recursive(&path, extension, out)?;
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e == extension)
        {
            out.push(path);
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn ensure_dir_creates_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        ensure_dir(&nested).unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn ensure_dir_existing_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dir(tmp.path()).unwrap(); // already exists
    }

    #[test]
    fn materialize_hardlink() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("dest.txt");
        fs::write(&src, b"data").unwrap();

        materialize(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"data");
    }

    #[test]
    fn materialize_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("sub").join("dir").join("dest.txt");
        fs::write(&src, b"data").unwrap();

        materialize(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"data");
    }

    #[test]
    fn materialize_overwrites_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("dest.txt");
        fs::write(&src, b"new").unwrap();
        fs::write(&dest, b"old").unwrap();

        materialize(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
    }

    #[test]
    fn remove_dir_all_if_exists_removes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("target");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file.txt"), b"x").unwrap();

        remove_dir_all_if_exists(&dir).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn remove_dir_all_if_exists_absent_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nonexistent");
        remove_dir_all_if_exists(&dir).unwrap();
    }

    #[test]
    fn collect_files_finds_and_sorts() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("src");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("b.kt"), b"").unwrap();
        fs::write(sub.join("a.kt"), b"").unwrap();
        fs::write(tmp.path().join("c.kt"), b"").unwrap();
        fs::write(tmp.path().join("readme.md"), b"").unwrap();

        let files = collect_files(tmp.path(), "kt").unwrap();
        assert_eq!(files.len(), 3);
        // Verify sorted
        for i in 0..files.len().saturating_sub(1) {
            assert!(files.get(i) <= files.get(i + 1));
        }
    }

    #[test]
    fn collect_files_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let files = collect_files(tmp.path(), "kt").unwrap();
        assert!(files.is_empty());
    }
}
