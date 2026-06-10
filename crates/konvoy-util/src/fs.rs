//! Filesystem utilities for Konvoy.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::error::UtilError;

/// Build a `UtilError::Io` mapping closure for the given path.
///
/// Avoids repeating `map_err(|source| UtilError::Io { path: ..., source })`
/// across every filesystem wrapper.
fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> UtilError + '_ {
    move |source| UtilError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Create a directory and all parent directories if they do not exist.
///
/// # Errors
/// Returns an error if the directory cannot be created.
pub fn ensure_dir(path: &Path) -> Result<(), UtilError> {
    std::fs::create_dir_all(path).map_err(io_err(path))
}

/// Copy `src` to `dest`, preferring a hard link for speed.
///
/// Falls back to a regular copy if hard linking fails (e.g. cross-device).
///
/// # Errors
/// Returns an error if both hard linking and copying fail.
pub fn materialize(src: &Path, dest: &Path) -> Result<(), UtilError> {
    if let Some(parent) = dest.parent() {
        ensure_dir(parent)?;
    }
    if dest.exists() {
        std::fs::remove_file(dest).map_err(io_err(dest))?;
    }
    if std::fs::hard_link(src, dest).is_err() {
        std::fs::copy(src, dest).map_err(io_err(dest))?;
    }
    Ok(())
}

/// Write `contents` to a file, creating it if it does not exist.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), UtilError> {
    std::fs::write(path, contents).map_err(io_err(path))
}

/// Read the entire contents of a file into a byte vector.
///
/// # Errors
/// Returns an error if the file cannot be read.
pub fn read_file(path: &Path) -> Result<Vec<u8>, UtilError> {
    std::fs::read(path).map_err(io_err(path))
}

/// Copy a file from `src` to `dest`.
///
/// # Errors
/// Returns an error if the copy fails.
pub fn copy_file(src: &Path, dest: &Path) -> Result<u64, UtilError> {
    std::fs::copy(src, dest).map_err(io_err(dest))
}

/// Rename (move) a file or directory from `from` to `to`.
///
/// # Errors
/// Returns an error if the rename fails.
pub fn rename(from: &Path, to: &Path) -> Result<(), UtilError> {
    std::fs::rename(from, to).map_err(io_err(to))
}

/// Remove a directory and all its contents. No error if the directory is absent.
///
/// # Errors
/// Returns an error if the directory exists but cannot be removed.
pub fn remove_dir_all_if_exists(path: &Path) -> Result<(), UtilError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_err(path)(source)),
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
        .map_err(|_| UtilError::NoHomeDir)?;
    Ok(home.join(".konvoy"))
}

/// Return an environment path with `entry` prepended before the existing entries.
///
/// This is intended for `PATH`-style environment variables. It uses the platform
/// separator through [`std::env::join_paths`] rather than hard-coding `:`.
///
/// # Errors
/// Returns an error if any path entry cannot be represented in a joined search path.
pub fn prepend_to_environment_path(
    entry: &Path,
    existing_path: Option<&OsStr>,
) -> Result<OsString, std::env::JoinPathsError> {
    let mut paths = vec![entry.to_path_buf()];
    if let Some(existing_path) = existing_path {
        paths.extend(std::env::split_paths(existing_path));
    }
    std::env::join_paths(paths)
}

/// Collect all files with the given `extension` under `dir`, recursively, sorted by path.
///
/// # Errors
/// Returns an error if `dir` cannot be read.
pub fn collect_files(dir: &Path, extension: &str) -> Result<Vec<PathBuf>, UtilError> {
    let mut files = Vec::new();
    collect_files_recursive(dir, Some(extension), &mut files)?;
    files.sort();
    Ok(files)
}

/// Collect every file under `dir`, recursively, sorted by path (no extension filter).
///
/// Symlinks to files are followed and collected; symlinks to *directories* are NOT
/// traversed (this is the cycle-safety guard — `read_dir` still follows `dir` itself
/// if it is a symlink, so a symlinked top-level directory works; only a symlinked
/// sub-directory encountered during the walk is skipped). A symlink whose target
/// is missing is skipped; other resolution errors (permissions, I/O) propagate.
/// Hidden/dotfiles are returned — filtering is the caller's responsibility.
///
/// # Errors
/// Returns an error if `dir` (or a symlink target, other than a missing one)
/// cannot be read.
pub fn collect_all_files(dir: &Path) -> Result<Vec<PathBuf>, UtilError> {
    let mut files = Vec::new();
    collect_files_recursive(dir, None, &mut files)?;
    files.sort();
    Ok(files)
}

/// Check whether a path has the given file extension (case-sensitive).
fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e == ext)
}

/// Recursively collect files under `dir`. When `extension` is `Some`, only files
/// with that extension are collected; when `None`, every file is collected.
fn collect_files_recursive(
    dir: &Path,
    extension: Option<&str>,
    out: &mut Vec<PathBuf>,
) -> Result<(), UtilError> {
    let entries = std::fs::read_dir(dir).map_err(io_err(dir))?;

    let matches = |path: &Path| extension.is_none_or(|ext| has_extension(path, ext));

    for entry in entries {
        let entry = entry.map_err(io_err(dir))?;

        // Use entry.file_type() which does NOT follow symlinks, unlike
        // path.is_dir()/path.metadata(). This prevents infinite recursion
        // when symlink cycles exist (e.g. a -> b, b -> a).
        let file_type = entry.file_type().map_err(io_err(&entry.path()))?;

        if file_type.is_symlink() {
            // Follow the symlink to classify its target (std::fs::metadata follows).
            // A genuinely broken symlink (target missing → NotFound) is skipped, but
            // any OTHER error (permission, I/O, a path component that isn't a
            // directory) is propagated — matching how an unreadable real directory
            // fails loudly via read_dir, so the collected set never silently changes
            // due to a transient/permission error (it feeds deterministic cache keys).
            let path = entry.path();
            let target_meta = match std::fs::metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_err(&path)(e)),
            };

            // Only include symlinked regular files; skip directory symlinks
            // to prevent infinite recursion from symlink cycles.
            if target_meta.is_file() && matches(&path) {
                out.push(path);
            }
            continue;
        }

        let path = entry.path();

        if file_type.is_dir() {
            collect_files_recursive(&path, extension, out)?;
        } else if matches(&path) {
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
    fn collect_all_files_returns_every_file_regardless_of_extension() {
        // Unlike collect_files, collect_all_files has no extension filter — it must
        // return files of any (or no) extension, recursively, sorted.
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("nested");
        fs::create_dir_all(&sub).unwrap();
        fs::write(tmp.path().join("api.yaml"), b"").unwrap();
        fs::write(tmp.path().join("README"), b"").unwrap(); // no extension
        fs::write(sub.join("pet.json"), b"").unwrap();

        let files = collect_all_files(tmp.path()).unwrap();
        assert_eq!(
            files,
            vec![
                tmp.path().join("README"),
                tmp.path().join("api.yaml"),
                sub.join("pet.json"),
            ]
        );
    }

    #[test]
    fn collect_all_files_empty_dir_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(collect_all_files(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn collect_all_files_includes_hidden_files_and_descends_dot_dirs() {
        // collect_all_files is intentionally UNFILTERED — callers (e.g. codegen)
        // decide what to exclude. It must return dotfiles and descend into
        // dot-directories; locking this in stops anyone "helpfully" adding hidden-
        // file filtering to the shared util.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".git")).unwrap();
        fs::write(tmp.path().join(".env"), b"secret").unwrap();
        fs::write(tmp.path().join(".git/config"), b"[core]").unwrap();
        fs::write(tmp.path().join("normal.txt"), b"x").unwrap();

        let files = collect_all_files(tmp.path()).unwrap();
        assert_eq!(
            files,
            vec![
                tmp.path().join(".env"),
                tmp.path().join(".git/config"),
                tmp.path().join("normal.txt"),
            ]
        );
    }

    #[test]
    fn collect_all_files_does_not_treat_a_directory_as_a_file() {
        // A directory whose name looks like a file is recursed into, not collected.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("looks_like.yaml")).unwrap();
        fs::write(tmp.path().join("looks_like.yaml/inner.yaml"), b"x").unwrap();

        let files = collect_all_files(tmp.path()).unwrap();
        assert_eq!(files, vec![tmp.path().join("looks_like.yaml/inner.yaml")]);
    }

    #[test]
    fn collect_all_files_errors_on_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let err = collect_all_files(&tmp.path().join("does-not-exist")).unwrap_err();
        assert!(
            matches!(err, UtilError::Io { .. }),
            "expected Io, got {err:?}"
        );
    }

    #[test]
    fn collect_all_files_sorts_results_lexically() {
        // Files are CREATED in scrambled order; the result must come back in lexical
        // (byte) order, proving the `.sort()` is applied rather than passing through
        // whatever order `read_dir` happens to yield. The `a10` < `a2` pair pins
        // lexical (not numeric) ordering. (Verified: reverting the sort fails this on
        // a real filesystem, whose read_dir order is not coincidentally sorted.)
        let tmp = tempfile::tempdir().unwrap();
        for name in ["d", "c", "b", "a2", "a10", "a1", "a"] {
            fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let files = collect_all_files(tmp.path()).unwrap();
        let expected: Vec<PathBuf> = ["a", "a1", "a10", "a2", "b", "c", "d"]
            .iter()
            .map(|n| tmp.path().join(n))
            .collect();
        assert_eq!(files, expected);
    }

    #[test]
    fn collect_all_files_is_a_superset_of_collect_files() {
        // Same tree, two modes: collect_files filters by extension; collect_all_files
        // returns everything (sorted). Documents the generalization's two behaviors.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.kt"), b"x").unwrap();
        fs::write(tmp.path().join("b.txt"), b"y").unwrap();
        fs::write(tmp.path().join("README"), b"z").unwrap();

        let kt = collect_files(tmp.path(), "kt").unwrap();
        let all = collect_all_files(tmp.path()).unwrap();
        assert_eq!(kt, vec![tmp.path().join("a.kt")]);
        assert_eq!(
            all,
            vec![
                tmp.path().join("README"),
                tmp.path().join("a.kt"),
                tmp.path().join("b.txt"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn collect_all_files_skips_broken_symlinks() {
        // A dangling symlink (target missing) must be silently skipped — neither
        // collected nor surfaced as an error — or a stray broken link would poison a
        // cache key built from the result, or abort the whole walk. Covers the
        // `Err(metadata) => continue` branch, which no other test exercises.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("real.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("missing"), tmp.path().join("dangling"))
            .unwrap();

        let files = collect_all_files(tmp.path()).unwrap();
        assert_eq!(files, vec![tmp.path().join("real.txt")]);
    }

    #[cfg(unix)]
    #[test]
    fn collect_all_files_propagates_non_missing_symlink_errors() {
        // A symlink that fails to resolve for a reason OTHER than "missing target"
        // must error, not be silently skipped like a genuinely-broken link. Here the
        // target path traverses a regular file (`blocker/inner`), so resolution fails
        // with ENOTDIR (not NotFound) — deterministically and independent of the test
        // user (unlike a permission-based case, which root would bypass). This guards
        // the determinism contract: transient/permission/IO errors fail loudly rather
        // than silently dropping a file from a cache-key input set.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("blocker"), b"x").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("blocker/inner"), tmp.path().join("bad"))
            .unwrap();

        let result = collect_all_files(tmp.path());
        assert!(
            matches!(result, Err(UtilError::Io { .. })),
            "a non-NotFound symlink-resolution error must propagate, got {result:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn collect_all_files_follows_file_symlinks_and_skips_dir_symlink_cycles() {
        // The extension->Option generalization rewrote the symlink branch
        // (`has_extension` -> `matches`); the existing symlink tests only cover the
        // extension path via collect_files. This covers the None path: a symlinked
        // (extension-less) FILE is followed and collected, while a directory-symlink
        // cycle is skipped (no hang / stack overflow).
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        // Directory-symlink cycle a <-> b: must be skipped, not followed.
        std::os::unix::fs::symlink(&dir_b, dir_a.join("to_b")).unwrap();
        std::os::unix::fs::symlink(&dir_a, dir_b.join("to_a")).unwrap();

        // A real extension-less file and an extension-less symlink to another file.
        let target = tmp.path().join("real_target");
        fs::write(&target, b"t").unwrap();
        fs::write(dir_a.join("plain"), b"p").unwrap();
        std::os::unix::fs::symlink(&target, dir_a.join("linked")).unwrap();

        // Assert the EXACT set (not just presence): this catches over-collection
        // (e.g. extra entries from a wrongly-followed dir symlink) and inherently
        // proves the dir-symlink cycle contributed nothing — the prior `any()` +
        // `!any(to_a/to_b)` checks couldn't catch either (the negative was a
        // tautology, since dir symlinks are never pushed). If the cycle-skip itself
        // regressed, the walk would loop (FilesystemLoop / overflow) before asserting.
        let files = collect_all_files(tmp.path()).unwrap();
        assert_eq!(
            files,
            vec![
                dir_a.join("linked"), // symlink to a file -> followed
                dir_a.join("plain"),  // real extension-less file
                tmp.path().join("real_target"),
            ]
        );
    }

    #[test]
    fn collect_files_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let files = collect_files(tmp.path(), "kt").unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn collect_files_skips_regular_files_without_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("src");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("main.kt"), b"fun main() {}").unwrap();
        fs::write(sub.join("helper.kt"), b"fun helper() {}").unwrap();
        fs::write(sub.join("notes.txt"), b"not kotlin").unwrap();
        fs::write(sub.join("data.json"), b"{}").unwrap();

        let files = collect_files(tmp.path(), "kt").unwrap();
        assert_eq!(files.len(), 2);
        assert!(files
            .iter()
            .all(|f| f.extension().and_then(|e| e.to_str()) == Some("kt")));
    }

    #[cfg(unix)]
    #[test]
    fn collect_files_skips_symlink_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        // Create a symlink cycle: a/link_to_b -> ../b, b/link_to_a -> ../a
        std::os::unix::fs::symlink(&dir_b, dir_a.join("link_to_b")).unwrap();
        std::os::unix::fs::symlink(&dir_a, dir_b.join("link_to_a")).unwrap();

        // Place a real file so we verify collection still works
        fs::write(dir_a.join("real.kt"), b"fun real() {}").unwrap();

        // This must complete without hanging or stack-overflowing
        let files = collect_files(tmp.path(), "kt").unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.first().unwrap().ends_with("real.kt"));
    }

    #[cfg(unix)]
    #[test]
    fn collect_files_follows_symlinked_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let real_dir = tmp.path().join("real");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&real_dir).unwrap();

        // Place real .kt files outside of src/
        fs::write(real_dir.join("alpha.kt"), b"fun alpha() {}").unwrap();
        fs::write(real_dir.join("beta.kt"), b"fun beta() {}").unwrap();
        fs::write(real_dir.join("gamma.txt"), b"not kotlin").unwrap();

        // Symlink .kt files into src/
        std::os::unix::fs::symlink(real_dir.join("alpha.kt"), src_dir.join("alpha.kt")).unwrap();
        std::os::unix::fs::symlink(real_dir.join("beta.kt"), src_dir.join("beta.kt")).unwrap();
        // Symlink a non-.kt file to ensure extension filtering still applies
        std::os::unix::fs::symlink(real_dir.join("gamma.txt"), src_dir.join("gamma.txt")).unwrap();

        let files = collect_files(&src_dir, "kt").unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("alpha.kt")));
        assert!(files.iter().any(|f| f.ends_with("beta.kt")));
    }

    // Shared with `pom::tests` and `module_metadata::tests` via
    // `crate::test_util::ENV_LOCK` so HOME-override tests in different
    // modules don't race when `cargo test` runs them on multiple threads.
    use crate::test_util::ENV_LOCK;

    #[test]
    fn konvoy_home_returns_dotkonvoy_subdir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = konvoy_home().unwrap();
        assert!(
            home.ends_with(".konvoy"),
            "expected path ending in .konvoy, got: {}",
            home.display()
        );
    }

    #[test]
    fn konvoy_home_fails_without_home_vars() {
        let _guard = ENV_LOCK.lock().unwrap();

        let saved_home = std::env::var("HOME").ok();
        let saved_profile = std::env::var("USERPROFILE").ok();
        std::env::remove_var("HOME");
        std::env::remove_var("USERPROFILE");

        let result = konvoy_home();

        // Restore before asserting.
        if let Some(v) = saved_home {
            std::env::set_var("HOME", v);
        }
        if let Some(v) = saved_profile {
            std::env::set_var("USERPROFILE", v);
        }

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("home directory"), "error was: {err}");
    }

    #[test]
    fn ensure_dir_deeply_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("x").join("y").join("z").join("w").join("v");
        ensure_dir(&deep).unwrap();
        assert!(deep.is_dir());
        // Calling again on the same path is a no-op.
        ensure_dir(&deep).unwrap();
        assert!(deep.is_dir());
    }

    #[test]
    fn materialize_content_matches_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("original.bin");
        let dest = tmp.path().join("copy.bin");
        let content = b"hello konvoy world\nline two\n";
        fs::write(&src, content).unwrap();

        materialize(&src, &dest).unwrap();

        // Verify byte-for-byte content match.
        assert_eq!(fs::read(&dest).unwrap(), content);
    }

    #[test]
    fn materialize_overwrites_larger_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dest = tmp.path().join("dest.txt");
        // Write a large old file, then a small new source.
        fs::write(
            &dest,
            b"this is a much longer old content that should be replaced",
        )
        .unwrap();
        fs::write(&src, b"short").unwrap();

        materialize(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"short");
    }

    #[test]
    fn remove_dir_all_if_exists_deeply_nested_non_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a").join("b").join("c");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("deep.txt"), b"deep content").unwrap();
        fs::write(
            tmp.path().join("a").join("b").join("mid.txt"),
            b"mid content",
        )
        .unwrap();

        // Remove from root of the subtree.
        remove_dir_all_if_exists(&tmp.path().join("a")).unwrap();
        assert!(!tmp.path().join("a").exists());
    }

    #[test]
    fn konvoy_home_uses_custom_home_var() {
        let _guard = ENV_LOCK.lock().unwrap();

        let saved_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", "/tmp/fake_home");

        let result = konvoy_home().unwrap();

        // Restore before asserting.
        if let Some(v) = saved_home {
            std::env::set_var("HOME", v);
        } else {
            std::env::remove_var("HOME");
        }

        assert_eq!(result, PathBuf::from("/tmp/fake_home/.konvoy"));
    }

    #[test]
    fn prepend_to_environment_path_adds_entry_before_existing_path() {
        let path = prepend_to_environment_path(
            Path::new("/shim"),
            Some(std::ffi::OsStr::new("/usr/bin:/bin")),
        )
        .unwrap();
        assert_eq!(path.to_string_lossy(), "/shim:/usr/bin:/bin");
    }

    #[test]
    fn prepend_to_environment_path_handles_missing_existing_path() {
        let path = prepend_to_environment_path(Path::new("/shim"), None).unwrap();
        assert_eq!(path.to_string_lossy(), "/shim");
    }

    #[test]
    fn prepend_to_environment_path_preserves_empty_existing_path_entry() {
        let path = prepend_to_environment_path(Path::new("/shim"), Some(std::ffi::OsStr::new("")))
            .unwrap();
        assert_eq!(path.to_string_lossy(), "/shim:");
    }

    #[test]
    fn prepend_to_environment_path_preserves_duplicate_entries() {
        let path = prepend_to_environment_path(
            Path::new("/shim"),
            Some(std::ffi::OsStr::new("/usr/bin:/shim:/usr/bin")),
        )
        .unwrap();
        assert_eq!(path.to_string_lossy(), "/shim:/usr/bin:/shim:/usr/bin");
    }

    #[test]
    fn prepend_to_environment_path_preserves_entries_with_spaces() {
        let path = prepend_to_environment_path(
            Path::new("/tmp/konvoy shim"),
            Some(std::ffi::OsStr::new(
                "/usr/local/bin:/Applications/My Tool/bin",
            )),
        )
        .unwrap();
        assert_eq!(
            path.to_string_lossy(),
            "/tmp/konvoy shim:/usr/local/bin:/Applications/My Tool/bin"
        );
    }

    #[test]
    fn collect_files_deeply_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("deep.kt"), b"fun deep() {}").unwrap();
        fs::write(tmp.path().join("top.kt"), b"fun top() {}").unwrap();

        let files = collect_files(tmp.path(), "kt").unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("deep.kt")));
        assert!(files.iter().any(|f| f.ends_with("top.kt")));
    }

    #[test]
    fn collect_files_no_matching_extension() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("file.rs"), b"fn main() {}").unwrap();
        fs::write(tmp.path().join("file.txt"), b"hello").unwrap();

        let files = collect_files(tmp.path(), "kt").unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn materialize_error_on_nonexistent_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("does_not_exist.txt");
        let dest = tmp.path().join("dest.txt");

        let result = materialize(&src, &dest);
        // hard_link fails, then copy fails too => error
        assert!(result.is_err());
    }
}
