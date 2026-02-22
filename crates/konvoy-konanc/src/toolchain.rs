//! Managed toolchain download, installation, and discovery.
//!
//! Downloads Kotlin/Native prebuilt tarballs from JetBrains GitHub releases
//! and installs them under `~/.konvoy/toolchains/<version>/`.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::KonancError;

/// Result of installing a managed toolchain.
#[derive(Debug, Clone)]
pub struct InstallResult {
    /// Absolute path to the `konanc` binary.
    pub konanc_path: PathBuf,
    /// SHA-256 hex digest of the downloaded Kotlin/Native tarball.
    pub konanc_tarball_sha256: String,
    /// Absolute path to the bundled JRE's JAVA_HOME directory.
    pub jre_home: PathBuf,
    /// SHA-256 hex digest of the downloaded JRE tarball.
    pub jre_tarball_sha256: String,
}

/// Return the root directory for managed toolchains: `~/.konvoy/toolchains/`.
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
pub fn toolchains_dir() -> Result<PathBuf, KonancError> {
    let home = home_dir()?;
    Ok(home.join(".konvoy").join("toolchains"))
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
                konanc_tarball_sha256: String::new(),
                jre_home,
                jre_tarball_sha256: String::new(),
            });
        }
        // konanc installed but JRE missing — fall through to install JRE only.
    }

    // --- Install konanc if needed ---
    let konanc_sha256 = if konanc_already_installed {
        String::new()
    } else {
        let url = download_url(version)?;
        let toolchains_root = toolchains_dir()?;

        // Ensure the toolchains directory exists.
        std::fs::create_dir_all(&toolchains_root).map_err(|source| KonancError::Io {
            path: toolchains_root.display().to_string(),
            source,
        })?;

        // Use randomized temp names to avoid collisions between concurrent installs.
        let suffix = temp_suffix();

        // Download to a temp file, computing SHA-256 as we go.
        let tmp_tarball = toolchains_root.join(format!(".tmp-{version}-{suffix}.tar.gz"));
        let sha256 = download_with_progress(&url, &tmp_tarball, "Kotlin/Native", version)?;

        // Extract to a temp directory, then rename atomically.
        let tmp_extract = toolchains_root.join(format!(".tmp-{version}-{suffix}-extract"));
        if tmp_extract.exists() {
            std::fs::remove_dir_all(&tmp_extract).map_err(|source| KonancError::Io {
                path: tmp_extract.display().to_string(),
                source,
            })?;
        }

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

        sha256
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
/// Returns `(jre_home, tarball_sha256)`. Skips download if already installed.
fn install_jre(version: &str) -> Result<(PathBuf, String), KonancError> {
    let jre_root = jre_dir(version)?;

    // Already installed — return existing path.
    if jre_root.exists() {
        let home = jre_home_path(version)?;
        return Ok((home, String::new()));
    }

    let url = jre_download_url()?;
    let toolchains_root = toolchains_dir()?;

    // Use randomized temp names to avoid collisions between concurrent installs.
    let suffix = temp_suffix();

    // Download JRE tarball.
    let tmp_tarball = toolchains_root.join(format!(".tmp-{version}-jre-{suffix}.tar.gz"));
    let sha256 = download_with_progress(&url, &tmp_tarball, "JRE", version)?;

    // Extract to temp directory.
    let tmp_extract = toolchains_root.join(format!(".tmp-{version}-jre-{suffix}-extract"));
    if tmp_extract.exists() {
        std::fs::remove_dir_all(&tmp_extract).map_err(|source| KonancError::Io {
            path: tmp_extract.display().to_string(),
            source,
        })?;
    }

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

    Ok((home, sha256))
}

/// Find the extracted JRE root directory inside the jre/ directory.
fn find_jre_root(jre_dir: &Path) -> Result<PathBuf, KonancError> {
    let entries: Vec<_> = std::fs::read_dir(jre_dir)
        .map_err(|source| KonancError::Io {
            path: jre_dir.display().to_string(),
            source,
        })?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .collect();

    if entries.len() == 1 {
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
        ("macos", "x86_64") => Ok(("mac", "x64")),
        ("macos", "aarch64") => Ok(("mac", "aarch64")),
        _ => Err(KonancError::UnsupportedPlatform {
            os: os.to_owned(),
            arch: arch.to_owned(),
        }),
    }
}

/// Construct the download URL for a Kotlin/Native prebuilt tarball.
fn download_url(version: &str) -> Result<String, KonancError> {
    let (os, arch) = platform_slug()?;
    Ok(format!(
        "https://github.com/JetBrains/kotlin/releases/download/v{version}/kotlin-native-prebuilt-{os}-{arch}-{version}.tar.gz"
    ))
}

/// Map the current OS and architecture to the JetBrains release slug.
fn platform_slug() -> Result<(&'static str, &'static str), KonancError> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("linux", "x86_64") => Ok(("linux", "x86_64")),
        ("macos", "x86_64") => Ok(("macos", "x86_64")),
        ("macos", "aarch64") => Ok(("macos", "aarch64")),
        _ => Err(KonancError::UnsupportedPlatform {
            os: os.to_owned(),
            arch: arch.to_owned(),
        }),
    }
}

/// Download a URL to a file, showing progress on stderr and computing SHA-256.
fn download_with_progress(
    url: &str,
    dest: &Path,
    label: &str,
    version: &str,
) -> Result<String, KonancError> {
    let agent = ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .timeout_connect(Some(std::time::Duration::from_secs(30)))
            .timeout_global(Some(std::time::Duration::from_secs(600)))
            .build(),
    );

    let response = agent.get(url).call().map_err(|e| KonancError::Download {
        version: version.to_owned(),
        message: e.to_string(),
    })?;

    let content_length: Option<u64> = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    let mut body = response.into_body();
    let mut file = std::fs::File::create(dest).map_err(|source| KonancError::Io {
        path: dest.display().to_string(),
        source,
    })?;

    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_pct: u8 = 0;
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = std::io::Read::read(&mut body.as_reader(), &mut buf).map_err(|e| {
            KonancError::Download {
                version: version.to_owned(),
                message: e.to_string(),
            }
        })?;
        if n == 0 {
            break;
        }

        let chunk = buf.get(..n).unwrap_or(&buf);
        std::io::Write::write_all(&mut file, chunk).map_err(|source| KonancError::Io {
            path: dest.display().to_string(),
            source,
        })?;
        hasher.update(chunk);

        downloaded = downloaded.saturating_add(n as u64);

        // Show progress.
        if let Some(total) = content_length {
            if total > 0 {
                #[allow(clippy::cast_possible_truncation)]
                let pct = ((downloaded * 100) / total) as u8;
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

/// Generate a unique suffix for temp file/directory names.
///
/// Combines the process ID with a random component to prevent collisions
/// between concurrent installs (even from the same PID via fork).
fn temp_suffix() -> String {
    let pid = std::process::id();
    let random: u32 = rand_u32();
    format!("{pid}-{random:08x}")
}

/// Simple random u32 using system time as a seed source.
/// Not cryptographically secure, but sufficient for temp name uniqueness.
fn rand_u32() -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    #[allow(clippy::cast_possible_truncation)]
    let result = hasher.finish() as u32;
    result
}

/// Get the user's home directory.
fn home_dir() -> Result<PathBuf, KonancError> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| KonancError::NoHomeDir)
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
                ("linux", "x64") | ("mac", "x64") | ("mac", "aarch64")
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
    fn temp_suffix_contains_pid() {
        let suffix = temp_suffix();
        let pid = std::process::id().to_string();
        assert!(
            suffix.starts_with(&pid),
            "suffix {suffix} should start with PID {pid}"
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
    fn temp_suffix_has_random_component() {
        // Two calls should produce different suffixes (random component differs).
        let a = temp_suffix();
        // Small sleep to ensure different time hash.
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = temp_suffix();
        assert_ne!(a, b, "consecutive temp suffixes should differ");
    }
}
