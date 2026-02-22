use std::path::PathBuf;
use std::process::Command;

/// Information about a detected `konanc` installation.
#[derive(Debug, Clone)]
pub struct KonancInfo {
    pub path: PathBuf,
    pub version: String,
}

/// Locate `konanc` and determine its version.
///
/// Checks `KONANC_HOME` env var first, then falls back to `PATH`.
pub fn detect_konanc() -> Result<KonancInfo, KonancError> {
    let path = if let Ok(home) = std::env::var("KONANC_HOME") {
        let p = PathBuf::from(home).join("bin").join("konanc");
        if !p.exists() {
            return Err(KonancError::NotFound);
        }
        p
    } else {
        which_konanc().ok_or(KonancError::NotFound)?
    };

    let output = Command::new(&path)
        .arg("-version")
        .output()
        .map_err(|e| KonancError::Exec { source: e })?;

    let version_str = String::from_utf8_lossy(&output.stdout);
    let version = version_str.trim().to_string();

    Ok(KonancInfo { path, version })
}

fn which_konanc() -> Option<PathBuf> {
    // Check if konanc is on PATH by trying to find it
    let output = Command::new("which").arg("konanc").output().ok()?;
    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout);
        Some(PathBuf::from(path_str.trim()))
    } else {
        None
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KonancError {
    #[error("konanc not found — install Kotlin/Native and add it to PATH, or set KONANC_HOME")]
    NotFound,
    #[error("failed to execute konanc: {source}")]
    Exec { source: std::io::Error },
}
