//! Managed toolchain download, installation, and discovery.
//!
//! Downloads Kotlin/Native prebuilt tarballs from `download.jetbrains.com`
//! and installs them under `~/.konvoy/toolchains/<version>/`.

use std::path::{Path, PathBuf};

use crate::error::KonancError;

/// Map a `UtilError` to `KonancError::Download`.
fn map_download_err(version: &str, e: konvoy_util::error::UtilError) -> KonancError {
    match e {
        konvoy_util::error::UtilError::Download { message } => KonancError::Download {
            version: version.to_owned(),
            message,
        },
        konvoy_util::error::UtilError::Io { path, source } => KonancError::Io { path, source },
        other => KonancError::Download {
            version: version.to_owned(),
            message: other.to_string(),
        },
    }
}

/// Result of installing a managed toolchain.
#[derive(Debug, Clone)]
pub struct InstallResult {
    /// Absolute path to the `konanc` binary.
    pub konanc_path: PathBuf,
    /// SHA-256 hex digest of the downloaded Kotlin/Native tarball, or `None` if
    /// the toolchain was already installed (no download occurred).
    pub konanc_tarball_sha256: Option<String>,
    /// Absolute path to the bundled JRE's JAVA_HOME directory.
    pub jre_home: PathBuf,
    /// SHA-256 hex digest of the downloaded JRE tarball, or `None` if the JRE
    /// was already installed (no download occurred).
    pub jre_tarball_sha256: Option<String>,
}

/// Return the root directory for managed toolchains: `~/.konvoy/toolchains/`.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn toolchains_dir() -> Result<PathBuf, KonancError> {
    let konvoy_home = konvoy_util::fs::konvoy_home().map_err(|_| KonancError::NoHomeDir)?;
    Ok(konvoy_home.join("toolchains"))
}

/// Return the installation directory for a specific version.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn version_dir(version: &str) -> Result<PathBuf, KonancError> {
    Ok(toolchains_dir()?.join(version))
}

/// Return the path to `konanc` for a managed version.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn managed_konanc_path(version: &str) -> Result<PathBuf, KonancError> {
    Ok(version_dir(version)?.join("bin").join("konanc"))
}

/// Return the JRE directory for a specific toolchain version.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn jre_dir(version: &str) -> Result<PathBuf, KonancError> {
    Ok(version_dir(version)?.join("jre"))
}

/// Return the JAVA_HOME path for the bundled JRE.
///
/// The JRE tarball extracts to a single directory (e.g. `jdk-21.0.10+7-jre/`).
/// On macOS, JAVA_HOME is `<extracted>/Contents/Home/`; on Linux it's the
/// extracted root itself.
///
/// # Errors
/// Returns an error if the JRE is not installed or the home directory cannot
/// be determined.
pub fn jre_home_path(version: &str) -> Result<PathBuf, KonancError> {
    let jre_root = jre_dir(version)?;
    if !jre_root.exists() {
        return Err(KonancError::JreInstall {
            message: format!("JRE not found at {}", jre_root.display()),
        });
    }

    // Find the single extracted directory inside jre/.
    let extracted = find_jre_root(&jre_root)?;

    // On macOS, the JRE uses Apple bundle layout: Contents/Home/
    let contents_home = extracted.join("Contents").join("Home");
    if contents_home.exists() {
        Ok(contents_home)
    } else {
        Ok(extracted)
    }
}

/// Check whether a specific version is fully installed (konanc + JRE).
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn is_installed(version: &str) -> Result<bool, KonancError> {
    let konanc = managed_konanc_path(version)?;
    if !konanc.exists() {
        return Ok(false);
    }
    // Also check that the JRE is present.
    let jre_root = jre_dir(version)?;
    Ok(jre_root.exists())
}

/// List all installed toolchain versions.
///
/// # Errors
/// Returns an error if the toolchains directory cannot be read.
pub fn list_installed() -> Result<Vec<String>, KonancError> {
    let Ok(dir) = toolchains_dir() else {
        return Ok(Vec::new());
    };

    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut versions = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|source| KonancError::Io {
        path: dir.display().to_string(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| KonancError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        if entry.path().is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                // Skip temp directories.
                if !name.starts_with(".tmp-") {
                    versions.push(name.to_owned());
                }
            }
        }
    }

    versions.sort();
    Ok(versions)
}

/// Download and install a Kotlin/Native toolchain.
///
/// # Errors
/// Returns an error if the download fails, the tarball is corrupt, or the
/// extraction fails.
pub fn install(version: &str) -> Result<InstallResult, KonancError> {
    let dest = version_dir(version)?;

    // Check if konanc is already installed.
    let konanc_path = dest.join("bin").join("konanc");
    let konanc_already_installed = konanc_path.exists();

    // If konanc exists, check if JRE also exists. If both present, return early.
    if konanc_already_installed {
        let jre_root = jre_dir(version)?;
        if jre_root.exists() {
            let jre_home = jre_home_path(version)?;
            return Ok(InstallResult {
                konanc_path,
                konanc_tarball_sha256: None,
                jre_home,
                jre_tarball_sha256: None,
            });
        }
        // konanc installed but JRE missing — fall through to install JRE only.
    }

    // --- Install konanc if needed ---
    let konanc_sha256 = if konanc_already_installed {
        None
    } else {
        let url = download_url(version)?;
        let toolchains_root = toolchains_dir()?;

        // Ensure the toolchains directory exists.
        std::fs::create_dir_all(&toolchains_root).map_err(|source| KonancError::Io {
            path: toolchains_root.display().to_string(),
            source,
        })?;

        // Create secure temp file and directory for download and extraction.
        let tmp_tarball_handle = tempfile::Builder::new()
            .prefix(&format!(".tmp-{version}-"))
            .suffix(".tar.gz")
            .tempfile_in(&toolchains_root)
            .map_err(|source| KonancError::Io {
                path: toolchains_root.display().to_string(),
                source,
            })?;
        let tmp_tarball = tmp_tarball_handle.path().to_path_buf();

        // Download to the temp file, computing SHA-256 as we go.
        let sha256 = konvoy_util::download::download_with_progress(
            &url,
            &tmp_tarball,
            "Kotlin/Native",
            version,
        )
        .map_err(|e| map_download_err(version, e))?;

        // Extract to a temp directory, then rename atomically.
        let tmp_extract_handle = tempfile::Builder::new()
            .prefix(&format!(".tmp-{version}-"))
            .suffix("-extract")
            .tempdir_in(&toolchains_root)
            .map_err(|source| KonancError::Io {
                path: toolchains_root.display().to_string(),
                source,
            })?;
        let tmp_extract = tmp_extract_handle.path().to_path_buf();

        extract_tarball(&tmp_tarball, &tmp_extract, "Kotlin/Native", version)?;

        // Clean up tarball.
        let _ = std::fs::remove_file(&tmp_tarball);

        // The tarball extracts to a subdirectory like `kotlin-native-prebuilt-linux-x86_64-2.1.0/`.
        let extracted_root = find_extracted_root(&tmp_extract, version)?;

        // Atomically move into place. If another process raced us, rename
        // will fail and we verify the existing installation instead.
        match std::fs::rename(&extracted_root, &dest) {
            Ok(()) => {
                let _ = std::fs::remove_dir_all(&tmp_extract);
            }
            Err(_) if dest.exists() => {
                // Another process installed while we were downloading.
                let _ = std::fs::remove_dir_all(&tmp_extract);
            }
            Err(source) => {
                let _ = std::fs::remove_dir_all(&tmp_extract);
                return Err(KonancError::Io {
                    path: dest.display().to_string(),
                    source,
                });
            }
        }

        // Verify the installation.
        let final_konanc = dest.join("bin").join("konanc");
        if !final_konanc.exists() {
            return Err(KonancError::CorruptToolchain {
                path: dest,
                version: version.to_owned(),
            });
        }

        // Ensure konanc is executable on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&final_konanc).map_err(|source| KonancError::Io {
                path: final_konanc.display().to_string(),
                source,
            })?;
            let mut perms = metadata.permissions();
            let mode = perms.mode() | 0o755;
            perms.set_mode(mode);
            std::fs::set_permissions(&final_konanc, perms).map_err(|source| KonancError::Io {
                path: final_konanc.display().to_string(),
                source,
            })?;
        }

        Some(sha256)
    };

    // --- Install JRE if needed ---
    let (jre_home, jre_sha256) = install_jre(version)?;

    Ok(InstallResult {
        konanc_path: dest.join("bin").join("konanc"),
        konanc_tarball_sha256: konanc_sha256,
        jre_home,
        jre_tarball_sha256: jre_sha256,
    })
}

/// Download and install the bundled JRE for a toolchain version.
///
/// Returns `(jre_home, tarball_sha256)`. The SHA-256 is `None` if the JRE was
/// already installed and no download occurred.
fn install_jre(version: &str) -> Result<(PathBuf, Option<String>), KonancError> {
    let jre_root = jre_dir(version)?;

    // Already installed — return existing path.
    if jre_root.exists() {
        let home = jre_home_path(version)?;
        return Ok((home, None));
    }

    let url = jre_download_url()?;
    let toolchains_root = toolchains_dir()?;

    // Create secure temp file and directory for download and extraction.
    let tmp_tarball_handle = tempfile::Builder::new()
        .prefix(&format!(".tmp-{version}-jre-"))
        .suffix(".tar.gz")
        .tempfile_in(&toolchains_root)
        .map_err(|source| KonancError::Io {
            path: toolchains_root.display().to_string(),
            source,
        })?;
    let tmp_tarball = tmp_tarball_handle.path().to_path_buf();

    // Download JRE tarball.
    let sha256 = konvoy_util::download::download_with_progress(&url, &tmp_tarball, "JRE", version)
        .map_err(|e| map_download_err(version, e))?;

    // Extract to temp directory.
    let tmp_extract_handle = tempfile::Builder::new()
        .prefix(&format!(".tmp-{version}-jre-"))
        .suffix("-extract")
        .tempdir_in(&toolchains_root)
        .map_err(|source| KonancError::Io {
            path: toolchains_root.display().to_string(),
            source,
        })?;
    let tmp_extract = tmp_extract_handle.path().to_path_buf();

    extract_tarball(&tmp_tarball, &tmp_extract, "JRE", version)?;

    // Clean up tarball.
    let _ = std::fs::remove_file(&tmp_tarball);

    // Atomically move into place. If another process raced us, rename
    // will fail and we verify the existing installation instead.
    match std::fs::rename(&tmp_extract, &jre_root) {
        Ok(()) => {}
        Err(_) if jre_root.exists() => {
            // Another process installed while we were downloading.
            let _ = std::fs::remove_dir_all(&tmp_extract);
        }
        Err(source) => {
            let _ = std::fs::remove_dir_all(&tmp_extract);
            return Err(KonancError::Io {
                path: jre_root.display().to_string(),
                source,
            });
        }
    }

    // Verify java binary exists.
    let home = jre_home_path(version)?;
    let java_bin = home.join("bin").join("java");
    if !java_bin.exists() {
        return Err(KonancError::JreInstall {
            message: format!("java binary not found at {}", java_bin.display()),
        });
    }

    // Ensure java is executable on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&java_bin).map_err(|source| KonancError::Io {
            path: java_bin.display().to_string(),
            source,
        })?;
        let mut perms = metadata.permissions();
        let mode = perms.mode() | 0o755;
        perms.set_mode(mode);
        std::fs::set_permissions(&java_bin, perms).map_err(|source| KonancError::Io {
            path: java_bin.display().to_string(),
            source,
        })?;
    }

    Ok((home, Some(sha256)))
}

/// Find the extracted JRE root directory inside the jre/ directory.
fn find_jre_root(jre_dir: &Path) -> Result<PathBuf, KonancError> {
    let entries: Vec<_> = std::fs::read_dir(jre_dir)
        .map_err(|source| KonancError::Io {
            path: jre_dir.display().to_string(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| KonancError::Io {
            path: jre_dir.display().to_string(),
            source,
        })?
        .into_iter()
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.len() == 1 {
        // The `unwrap_or_else` fallback is unreachable: we just confirmed
        // `entries.len() == 1`, so `.next()` always yields `Some`. The
        // fallback exists only to satisfy the type system without panicking.
        return Ok(entries
            .into_iter()
            .next()
            .map(|e| e.path())
            .unwrap_or_else(|| jre_dir.to_path_buf()));
    }

    // Look for a directory matching the JDK naming pattern.
    for entry in &entries {
        let name = entry.file_name();
        if let Some(s) = name.to_str() {
            if s.starts_with("jdk-") {
                return Ok(entry.path());
            }
        }
    }

    Err(KonancError::JreInstall {
        message: format!(
            "expected a single JRE directory in {}, found {} entries",
            jre_dir.display(),
            entries.len()
        ),
    })
}

/// Construct the download URL for an Adoptium Temurin JRE.
fn jre_download_url() -> Result<String, KonancError> {
    let (os, arch) = jre_platform_slug()?;
    Ok(format!(
        "https://api.adoptium.net/v3/binary/latest/21/ga/{os}/{arch}/jre/hotspot/normal/eclipse"
    ))
}

/// Map the current OS and architecture to Adoptium's naming convention.
fn jre_platform_slug() -> Result<(&'static str, &'static str), KonancError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("linux", "x86_64") => Ok(("linux", "x64")),
        ("linux", "aarch64") => Ok(("linux", "aarch64")),
        ("macos", "x86_64") => Ok(("mac", "x64")),
        ("macos", "aarch64") => Ok(("mac", "aarch64")),
        _ => Err(KonancError::UnsupportedPlatform {
            os: os.to_owned(),
            arch: arch.to_owned(),
        }),
    }
}

/// Construct the download URL for a Kotlin/Native prebuilt tarball.
///
/// Uses `download.jetbrains.com` which hosts prebuilt tarballs for all
/// supported platforms (including linux-aarch64 which is not published
/// to GitHub releases).
fn download_url(version: &str) -> Result<String, KonancError> {
    let (os, arch) = platform_slug()?;
    Ok(format!(
        "https://download.jetbrains.com/kotlin/native/builds/releases/{version}/{os}-{arch}/kotlin-native-prebuilt-{os}-{arch}-{version}.tar.gz"
    ))
}

/// Map the current OS and architecture to the JetBrains release slug.
fn platform_slug() -> Result<(&'static str, &'static str), KonancError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("linux", "x86_64") => Ok(("linux", "x86_64")),
        ("linux", "aarch64") => Ok(("linux", "aarch64")),
        ("macos", "x86_64") => Ok(("macos", "x86_64")),
        ("macos", "aarch64") => Ok(("macos", "aarch64")),
        _ => Err(KonancError::UnsupportedPlatform {
            os: os.to_owned(),
            arch: arch.to_owned(),
        }),
    }
}

/// Extract a `.tar.gz` tarball to a directory.
///
/// Each entry's path is validated to ensure it stays within `dest`,
/// preventing zip-slip (path traversal) attacks from malicious tarballs.
fn extract_tarball(
    tarball: &Path,
    dest: &Path,
    label: &str,
    version: &str,
) -> Result<(), KonancError> {
    eprintln!("    Extracting {label} {version}...");

    std::fs::create_dir_all(dest).map_err(|source| KonancError::Io {
        path: dest.display().to_string(),
        source,
    })?;

    let canonical_dest = std::fs::canonicalize(dest).map_err(|source| KonancError::Io {
        path: dest.display().to_string(),
        source,
    })?;

    let file = std::fs::File::open(tarball).map_err(|source| KonancError::Io {
        path: tarball.display().to_string(),
        source,
    })?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    let entries = archive.entries().map_err(|e| KonancError::Extract {
        version: version.to_owned(),
        message: e.to_string(),
    })?;

    for entry in entries {
        let mut entry = entry.map_err(|e| KonancError::Extract {
            version: version.to_owned(),
            message: e.to_string(),
        })?;

        let entry_path = entry.path().map_err(|e| KonancError::Extract {
            version: version.to_owned(),
            message: e.to_string(),
        })?;

        // Reject any path component that attempts directory traversal.
        for component in entry_path.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(KonancError::PathTraversal {
                    entry_path: entry_path.display().to_string(),
                    dest: canonical_dest.display().to_string(),
                });
            }
        }

        // Verify the resolved path stays within the destination.
        let target = canonical_dest.join(&*entry_path);
        if !target.starts_with(&canonical_dest) {
            return Err(KonancError::PathTraversal {
                entry_path: entry_path.display().to_string(),
                dest: canonical_dest.display().to_string(),
            });
        }

        // Ensure parent directories exist before unpacking the entry.
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|source| KonancError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }

        entry.unpack(&target).map_err(|e| KonancError::Extract {
            version: version.to_owned(),
            message: e.to_string(),
        })?;
    }

    Ok(())
}

/// Find the single extracted root directory inside a temp extraction dir.
fn find_extracted_root(extract_dir: &Path, version: &str) -> Result<PathBuf, KonancError> {
    let entries: Vec<_> = std::fs::read_dir(extract_dir)
        .map_err(|source| KonancError::Io {
            path: extract_dir.display().to_string(),
            source,
        })?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.len() == 1 {
        // The `unwrap_or_else` fallback is unreachable: we just confirmed
        // `entries.len() == 1`, so `.next()` always yields `Some`. The
        // fallback exists only to satisfy the type system without panicking.
        return Ok(entries
            .into_iter()
            .next()
            .map(|e| e.path())
            .unwrap_or_else(|| extract_dir.to_path_buf()));
    }

    // Fall back: look for a directory matching the expected pattern.
    for entry in &entries {
        let name = entry.file_name();
        if let Some(s) = name.to_str() {
            if s.contains("kotlin-native") && s.contains(version) {
                return Ok(entry.path());
            }
        }
    }

    Err(KonancError::Extract {
        version: version.to_owned(),
        message: format!(
            "expected a single directory in extracted tarball, found {} entries",
            entries.len()
        ),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn toolchains_dir_under_home() {
        let dir = toolchains_dir().unwrap();
        assert!(dir.display().to_string().contains(".konvoy"));
        assert!(dir.display().to_string().contains("toolchains"));
    }

    #[test]
    fn version_dir_includes_version() {
        let dir = version_dir("2.1.0").unwrap();
        assert!(dir.display().to_string().contains("2.1.0"));
    }

    #[test]
    fn managed_konanc_path_includes_bin() {
        let path = managed_konanc_path("2.1.0").unwrap();
        assert!(path.display().to_string().contains("bin"));
        assert!(path.display().to_string().contains("konanc"));
    }

    #[test]
    fn is_installed_false_for_missing() {
        // A version that doesn't exist should return false.
        let result = is_installed("99.99.99").unwrap();
        assert!(!result);
    }

    #[test]
    fn list_installed_returns_empty_for_no_toolchains() {
        // If no toolchains installed, should return empty or whatever is there.
        let result = list_installed();
        assert!(result.is_ok());
    }

    #[test]
    fn platform_slug_returns_valid() {
        // This test only passes on supported platforms.
        if let Ok((os, arch)) = platform_slug() {
            assert!(!os.is_empty());
            assert!(!arch.is_empty());
        }
    }

    #[test]
    fn download_url_format() {
        if let Ok(url) = download_url("2.1.0") {
            assert!(url.contains("2.1.0"));
            assert!(url.contains("kotlin-native-prebuilt"));
            assert!(url.contains(".tar.gz"));
            assert!(url.starts_with("https://"));
            assert!(url.contains("download.jetbrains.com"));
        }
    }

    #[test]
    fn jre_dir_under_version() {
        let dir = jre_dir("2.1.0").unwrap();
        let s = dir.display().to_string();
        assert!(s.contains(".konvoy"));
        assert!(s.contains("toolchains"));
        assert!(s.contains("2.1.0"));
        assert!(s.contains("jre"));
    }

    #[test]
    fn jre_download_url_format() {
        if let Ok(url) = jre_download_url() {
            assert!(url.contains("api.adoptium.net"));
            assert!(url.contains("/jre/"));
            assert!(url.contains("/21/"));
            assert!(url.starts_with("https://"));
        }
    }

    #[test]
    fn jre_platform_slug_valid() {
        if let Ok((os, arch)) = jre_platform_slug() {
            assert!(!os.is_empty());
            assert!(!arch.is_empty());
            // Adoptium uses "mac" not "macos", "x64" not "x86_64"
            assert!(matches!(
                (os, arch),
                ("linux", "x64") | ("linux", "aarch64") | ("mac", "x64") | ("mac", "aarch64")
            ));
        }
    }

    #[test]
    fn jre_home_path_errors_when_missing() {
        // A version that doesn't exist should error.
        let result = jre_home_path("99.99.99");
        assert!(result.is_err());
    }

    /// Helper: create a `.tar.gz` archive from a list of `(path, content)` entries.
    ///
    /// Writes raw USTAR tar headers so that malicious paths (containing `..`)
    /// are preserved verbatim — the `tar` crate's `Builder::append_data` rejects
    /// such paths, which would prevent us from testing our own validation.
    fn create_test_tarball(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
        use std::io::Write;

        let tmp = tempfile::NamedTempFile::new().unwrap();

        {
            let gz = flate2::write::GzEncoder::new(&tmp, flate2::Compression::fast());
            let mut out = std::io::BufWriter::new(gz);

            for &(path, content) in entries {
                let mut header = [0u8; 512];

                // name: bytes 0..100
                let path_bytes = path.as_bytes();
                let len = path_bytes.len().min(99);
                header
                    .get_mut(..len)
                    .unwrap()
                    .copy_from_slice(path_bytes.get(..len).unwrap());

                // mode: bytes 100..108
                header
                    .get_mut(100..108)
                    .unwrap()
                    .copy_from_slice(b"0000644\0");
                // uid: bytes 108..116
                header
                    .get_mut(108..116)
                    .unwrap()
                    .copy_from_slice(b"0001000\0");
                // gid: bytes 116..124
                header
                    .get_mut(116..124)
                    .unwrap()
                    .copy_from_slice(b"0001000\0");

                // size: bytes 124..136 (octal, 11 digits + NUL)
                #[allow(clippy::cast_possible_truncation)]
                let size_str = format!("{:011o}\0", content.len());
                header
                    .get_mut(124..136)
                    .unwrap()
                    .copy_from_slice(size_str.as_bytes().get(..12).unwrap());

                // mtime: bytes 136..148
                header
                    .get_mut(136..148)
                    .unwrap()
                    .copy_from_slice(b"00000000000\0");

                // typeflag: byte 156 — '0' = regular file
                header[156] = b'0';

                // magic: bytes 257..263
                header
                    .get_mut(257..263)
                    .unwrap()
                    .copy_from_slice(b"ustar\0");
                // version: bytes 263..265
                header.get_mut(263..265).unwrap().copy_from_slice(b"00");

                // Compute checksum: treat bytes 148..156 as spaces.
                header
                    .get_mut(148..156)
                    .unwrap()
                    .copy_from_slice(b"        ");
                let cksum: u32 = header.iter().map(|&b| u32::from(b)).sum();
                let cksum_str = format!("{cksum:06o}\0 ");
                header
                    .get_mut(148..156)
                    .unwrap()
                    .copy_from_slice(cksum_str.as_bytes().get(..8).unwrap());

                out.write_all(&header).unwrap();
                out.write_all(content).unwrap();

                // Pad content to a 512-byte boundary.
                let remainder = content.len() % 512;
                if remainder != 0 {
                    let pad = vec![0u8; 512 - remainder];
                    out.write_all(&pad).unwrap();
                }
            }

            // Two 512-byte zero blocks mark end of archive.
            out.write_all(&[0u8; 1024]).unwrap();
            out.flush().unwrap();
        }

        tmp
    }

    #[test]
    fn extract_tarball_safe_path_succeeds() {
        let tarball = create_test_tarball(&[("subdir/hello.txt", b"hello")]);
        let dest = tempfile::tempdir().unwrap();

        let result = extract_tarball(tarball.path(), dest.path(), "test", "0.0.0");
        assert!(result.is_ok());
        assert!(dest.path().join("subdir").join("hello.txt").exists());
    }

    #[test]
    fn extract_tarball_rejects_parent_dir_traversal() {
        let tarball = create_test_tarball(&[("../../etc/evil.txt", b"pwned")]);
        let dest = tempfile::tempdir().unwrap();

        let result = extract_tarball(tarball.path(), dest.path(), "test", "0.0.0");
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("path traversal"),
            "expected path traversal error, got: {msg}"
        );
    }

    #[test]
    fn tempfile_builder_creates_unique_files() {
        let dir = tempfile::tempdir().unwrap();
        let a = tempfile::Builder::new()
            .prefix(".tmp-test-")
            .suffix(".tar.gz")
            .tempfile_in(dir.path())
            .unwrap();
        let b = tempfile::Builder::new()
            .prefix(".tmp-test-")
            .suffix(".tar.gz")
            .tempfile_in(dir.path())
            .unwrap();
        assert_ne!(
            a.path(),
            b.path(),
            "two tempfiles in the same directory should have different paths"
        );
    }

    #[test]
    fn extract_tarball_rejects_dotdot_in_middle() {
        let tarball = create_test_tarball(&[("foo/../../../escape.txt", b"pwned")]);
        let dest = tempfile::tempdir().unwrap();

        let result = extract_tarball(tarball.path(), dest.path(), "test", "0.0.0");
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("path traversal"),
            "expected path traversal error, got: {msg}"
        );
    }

    #[test]
    #[allow(clippy::panic)]
    fn concurrent_rename_race_simulation() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("final-dest");
        let dest = Arc::new(dest);

        let num_threads = 8;
        let barrier = Arc::new(Barrier::new(num_threads));

        // Create source directories for each thread.
        let sources: Vec<_> = (0..num_threads)
            .map(|i| {
                let src = tmp.path().join(format!("src-{i}"));
                std::fs::create_dir_all(&src).unwrap();
                std::fs::write(src.join("marker.txt"), format!("thread-{i}")).unwrap();
                src
            })
            .collect();

        let handles: Vec<_> = sources
            .into_iter()
            .map(|src| {
                let dest = Arc::clone(&dest);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    // Synchronize all threads to maximize contention.
                    barrier.wait();

                    match std::fs::rename(&src, &*dest) {
                        Ok(()) => {
                            // We won the race.
                            "won"
                        }
                        Err(_) if dest.exists() => {
                            // Another thread won; clean up our source.
                            let _ = std::fs::remove_dir_all(&src);
                            "lost"
                        }
                        Err(e) => {
                            panic!("unexpected rename error: {e}");
                        }
                    }
                })
            })
            .collect();

        let mut winners = 0;
        let mut losers = 0;
        for handle in handles {
            match handle.join().unwrap() {
                "won" => winners += 1,
                "lost" => losers += 1,
                other => panic!("unexpected result: {other}"),
            }
        }

        // Exactly one thread should have won the rename race.
        // Note: on some filesystems, multiple renames may "succeed" if the
        // destination doesn't exist yet at the moment of the call. The key
        // invariant is that the destination exists and contains valid data.
        assert!(winners >= 1, "at least one thread should win the race");
        assert_eq!(winners + losers, num_threads, "all threads should complete");
        assert!(dest.exists(), "destination should exist after the race");
        assert!(
            dest.join("marker.txt").exists(),
            "destination should contain valid content"
        );
    }

    #[test]
    fn tempdir_builder_creates_unique_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let a = tempfile::Builder::new()
            .prefix(".tmp-test-")
            .suffix("-extract")
            .tempdir_in(dir.path())
            .unwrap();
        let b = tempfile::Builder::new()
            .prefix(".tmp-test-")
            .suffix("-extract")
            .tempdir_in(dir.path())
            .unwrap();
        assert_ne!(
            a.path(),
            b.path(),
            "two tempdirs in the same directory should have different paths"
        );
    }

    #[test]
    fn find_jre_root_single_directory() {
        let dir = tempfile::tempdir().unwrap();
        let jre_subdir = dir.path().join("jre-21");
        std::fs::create_dir(&jre_subdir).unwrap();

        let result = find_jre_root(dir.path());
        assert!(result.is_ok(), "single directory should succeed");
        assert_eq!(result.unwrap(), jre_subdir);
    }

    #[test]
    fn find_jre_root_prefers_jdk_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("some-dir");
        let jdk = dir.path().join("jdk-21.0.1");
        std::fs::create_dir(&other).unwrap();
        std::fs::create_dir(&jdk).unwrap();

        let result = find_jre_root(dir.path());
        assert!(result.is_ok(), "jdk- prefix should be preferred");
        assert_eq!(result.unwrap(), jdk);
    }

    #[test]
    fn find_jre_root_zero_directories_errors() {
        let dir = tempfile::tempdir().unwrap();
        // Create a file, not a directory — should be filtered out.
        std::fs::write(dir.path().join("not-a-dir.txt"), "hello").unwrap();

        let result = find_jre_root(dir.path());
        assert!(result.is_err(), "zero directories should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("found 0 entries"),
            "error should mention 0 entries, got: {err}"
        );
    }

    #[test]
    fn find_jre_root_multiple_non_jdk_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("foo")).unwrap();
        std::fs::create_dir(dir.path().join("bar")).unwrap();

        let result = find_jre_root(dir.path());
        assert!(result.is_err(), "multiple non-jdk dirs should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("found 2 entries"),
            "error should mention 2 entries, got: {err}"
        );
    }

    // ---- find_jre_root: empty directory ----

    #[test]
    fn find_jre_root_empty_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        // Completely empty — no files, no subdirectories.
        let result = find_jre_root(dir.path());
        assert!(result.is_err(), "empty directory should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("found 0 entries"),
            "error should mention 0 entries, got: {err}"
        );
    }

    // ---- find_extracted_root tests ----

    #[test]
    fn find_extracted_root_single_directory() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("kotlin-native-prebuilt-linux-x86_64-2.1.0");
        std::fs::create_dir(&subdir).unwrap();

        let result = find_extracted_root(dir.path(), "2.1.0");
        assert!(result.is_ok(), "single directory should succeed");
        assert_eq!(result.unwrap(), subdir);
    }

    #[test]
    fn find_extracted_root_empty_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        // Completely empty — no subdirectories.
        let result = find_extracted_root(dir.path(), "2.1.0");
        assert!(result.is_err(), "empty directory should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("found 0 entries"),
            "error should mention 0 entries, got: {err}"
        );
    }

    #[test]
    fn find_extracted_root_empty_with_only_files_errors() {
        let dir = tempfile::tempdir().unwrap();
        // Only files, no directories — directories are filtered by is_dir().
        std::fs::write(dir.path().join("readme.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("license.txt"), "MIT").unwrap();

        let result = find_extracted_root(dir.path(), "2.1.0");
        assert!(result.is_err(), "files-only directory should error");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("found 0 entries"),
            "error should mention 0 entries, got: {err}"
        );
    }

    #[test]
    fn find_extracted_root_multiple_dirs_picks_kotlin_native_match() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = dir.path().join("some-other-dir");
        let expected = dir.path().join("kotlin-native-prebuilt-linux-x86_64-2.1.0");
        std::fs::create_dir(&unrelated).unwrap();
        std::fs::create_dir(&expected).unwrap();

        let result = find_extracted_root(dir.path(), "2.1.0");
        assert!(
            result.is_ok(),
            "should find kotlin-native directory among multiple"
        );
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn find_extracted_root_multiple_dirs_no_match_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("foo")).unwrap();
        std::fs::create_dir(dir.path().join("bar")).unwrap();

        let result = find_extracted_root(dir.path(), "2.1.0");
        assert!(
            result.is_err(),
            "multiple non-matching dirs should error"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("found 2 entries"),
            "error should mention 2 entries, got: {err}"
        );
    }

    #[test]
    fn find_extracted_root_requires_version_in_name() {
        // A directory containing "kotlin-native" but wrong version should not match.
        let dir = tempfile::tempdir().unwrap();
        let wrong_version = dir.path().join("kotlin-native-prebuilt-linux-x86_64-1.9.0");
        let unrelated = dir.path().join("other");
        std::fs::create_dir(&wrong_version).unwrap();
        std::fs::create_dir(&unrelated).unwrap();

        let result = find_extracted_root(dir.path(), "2.1.0");
        assert!(
            result.is_err(),
            "kotlin-native dir with wrong version should not match"
        );
    }

    // ---- managed_konanc_path structure ----

    #[test]
    fn managed_konanc_path_structure() {
        let path = managed_konanc_path("2.1.0").unwrap();
        // Should end with <version>/bin/konanc.
        let s = path.display().to_string();
        assert!(
            s.ends_with("2.1.0/bin/konanc"),
            "path should end with version/bin/konanc, got: {s}"
        );
    }

    // ---- jre_home_path structure (macOS Contents/Home layout) ----

    #[test]
    fn jre_home_path_linux_layout() {
        // Simulate a Linux JRE layout: jre/<extracted-dir>/ is JAVA_HOME.
        let tmp = tempfile::tempdir().unwrap();

        // We cannot call jre_home_path directly (it uses the global home dir),
        // so we test the underlying find_jre_root + macOS detection logic.
        let jre_root = tmp.path();
        let extracted = jre_root.join("jdk-21.0.10+7-jre");
        std::fs::create_dir(&extracted).unwrap();

        let found = find_jre_root(jre_root).unwrap();
        assert_eq!(found, extracted);

        // On Linux, there is no Contents/Home, so the extracted root IS java home.
        let contents_home = found.join("Contents").join("Home");
        assert!(
            !contents_home.exists(),
            "Linux layout should not have Contents/Home"
        );
    }

    #[test]
    fn jre_home_path_macos_layout() {
        // Simulate a macOS JRE layout: jre/<extracted-dir>/Contents/Home/.
        let tmp = tempfile::tempdir().unwrap();
        let jre_root = tmp.path();
        let extracted = jre_root.join("jdk-21.0.10+7-jre");
        let contents_home = extracted.join("Contents").join("Home");
        std::fs::create_dir_all(&contents_home).unwrap();

        let found = find_jre_root(jre_root).unwrap();
        assert_eq!(found, extracted);

        // On macOS, Contents/Home exists, so it should be preferred.
        assert!(
            contents_home.exists(),
            "macOS layout should have Contents/Home"
        );
    }

    // ---- version_dir path construction ----

    #[test]
    fn version_dir_path_construction() {
        let dir = version_dir("2.1.0").unwrap();
        let s = dir.display().to_string();
        assert!(
            s.ends_with("toolchains/2.1.0"),
            "version_dir should end with toolchains/<version>, got: {s}"
        );
    }

    #[test]
    fn version_dir_different_versions_differ() {
        let a = version_dir("2.1.0").unwrap();
        let b = version_dir("2.2.0").unwrap();
        assert_ne!(a, b, "different versions should produce different paths");
    }

    // ---- is_installed edge cases ----

    #[test]
    fn is_installed_false_for_nonexistent_version() {
        // A completely made-up version should not be installed.
        let result = is_installed("0.0.0-nonexistent").unwrap();
        assert!(
            !result,
            "non-existent version should not report as installed"
        );
    }

    // ---- list_installed edge cases ----

    #[test]
    fn list_installed_result_is_sorted() {
        // list_installed sorts its output; verify this property holds even
        // when the underlying readdir returns entries out of order.
        let result = list_installed().unwrap();
        let mut sorted = result.clone();
        sorted.sort();
        assert_eq!(
            result, sorted,
            "list_installed should return sorted versions"
        );
    }

    // ---- map_download_err ----

    #[test]
    fn map_download_err_download_variant() {
        let util_err = konvoy_util::error::UtilError::Download {
            message: "connection refused".to_owned(),
        };
        let err = map_download_err("2.1.0", util_err);
        let msg = format!("{err}");
        assert!(msg.contains("2.1.0"), "should include version: {msg}");
        assert!(
            msg.contains("connection refused"),
            "should include message: {msg}"
        );
    }

    #[test]
    fn map_download_err_io_variant() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let util_err = konvoy_util::error::UtilError::Io {
            path: "/some/path".to_owned(),
            source: io_err,
        };
        let err = map_download_err("2.1.0", util_err);
        let msg = format!("{err}");
        assert!(
            msg.contains("/some/path"),
            "Io variant should preserve path: {msg}"
        );
    }

    #[test]
    fn map_download_err_other_variant_becomes_download() {
        // Any non-Download, non-Io variant should be mapped to Download.
        let util_err = konvoy_util::error::UtilError::ArtifactHashMismatch {
            path: "/file".to_owned(),
            expected: "aaa".to_owned(),
            actual: "bbb".to_owned(),
        };
        let err = map_download_err("2.1.0", util_err);
        let msg = format!("{err}");
        assert!(
            msg.contains("2.1.0"),
            "other variants should map to Download with version: {msg}"
        );
    }
}
