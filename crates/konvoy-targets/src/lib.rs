//! Host detection and target triple mapping for Konvoy.

use std::fmt;

/// A Kotlin/Native compilation target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    pub triple: String,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.triple)
    }
}

/// Detect the host target triple for Kotlin/Native.
///
/// Maps the Rust compile-time target to the Kotlin/Native target name.
///
/// # Errors
/// Returns an error if the current OS/arch is not supported by Kotlin/Native.
pub fn host_target() -> Result<Target, TargetError> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux_x64",
        ("linux", "aarch64") => "linux_arm64",
        ("macos", "x86_64") => "macos_x64",
        ("macos", "aarch64") => "macos_arm64",
        (os, arch) => {
            return Err(TargetError::UnsupportedHost {
                os: os.to_owned(),
                arch: arch.to_owned(),
            })
        }
    };
    Ok(Target {
        triple: triple.to_owned(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    #[error("unsupported host: {os}/{arch} — Kotlin/Native does not support this platform")]
    UnsupportedHost { os: String, arch: String },
}
