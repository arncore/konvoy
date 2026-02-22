#![forbid(unsafe_code)]
//! Host detection and target triple mapping for Konvoy.
//!
//! Maps Rust compile-time platform information to Kotlin/Native target names
//! and provides validation for user-supplied `--target` flags.

use std::fmt;
use std::str::FromStr;

/// All known Kotlin/Native targets supported by Konvoy.
const KNOWN_TARGETS: &[&str] = &["linux_x64", "linux_arm64", "macos_x64", "macos_arm64"];

/// A Kotlin/Native compilation target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Target {
    triple: String,
}

impl Target {
    /// Returns the string passed to `konanc -target`.
    pub fn to_konanc_arg(&self) -> &str {
        &self.triple
    }

    /// Returns `true` if this target matches the current host platform.
    ///
    /// # Errors
    /// Returns an error if the current host platform is unsupported.
    pub fn is_host(&self) -> Result<bool, TargetError> {
        let host = host_target()?;
        Ok(self == &host)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.triple)
    }
}

impl FromStr for Target {
    type Err = TargetError;

    /// Parse and validate a user-supplied target string.
    ///
    /// # Errors
    /// Returns `TargetError::InvalidTarget` if the string is not a known Kotlin/Native target.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if KNOWN_TARGETS.contains(&s) {
            Ok(Target {
                triple: s.to_owned(),
            })
        } else {
            Err(TargetError::InvalidTarget { name: s.to_owned() })
        }
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

    #[error(
        "unknown target `{name}`, supported targets: {}",
        KNOWN_TARGETS.join(", ")
    )]
    InvalidTarget { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_target_returns_valid_known_target() {
        let target = host_target();
        // On any supported CI/dev machine this should succeed
        let target = match target {
            Ok(t) => t,
            Err(_) => return, // skip on unsupported platforms
        };
        assert!(
            KNOWN_TARGETS.contains(&target.to_konanc_arg()),
            "host_target() returned `{}` which is not in KNOWN_TARGETS",
            target
        );
    }

    #[test]
    fn host_target_display_matches_triple() {
        let target = match host_target() {
            Ok(t) => t,
            Err(_) => return,
        };
        assert_eq!(target.to_string(), target.to_konanc_arg());
    }

    #[test]
    fn from_str_accepts_all_known_targets() {
        for &name in KNOWN_TARGETS {
            let target = Target::from_str(name);
            assert!(target.is_ok(), "from_str rejected known target `{name}`");
            let target = match target {
                Ok(t) => t,
                Err(_) => continue,
            };
            assert_eq!(target.to_konanc_arg(), name);
        }
    }

    #[test]
    fn from_str_rejects_invalid_target() {
        let result = Target::from_str("windows_x64");
        assert!(result.is_err());
    }

    #[test]
    fn from_str_rejects_empty_string() {
        let result = Target::from_str("");
        assert!(result.is_err());
    }

    #[test]
    fn invalid_target_error_lists_supported_targets() {
        let err = Target::from_str("bsd_x64");
        let msg = match err {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"), // only in test code
        };
        for &name in KNOWN_TARGETS {
            assert!(
                msg.contains(name),
                "error message should list `{name}`, got: {msg}"
            );
        }
    }

    #[test]
    fn display_format_matches_triple() {
        let target = match Target::from_str("linux_x64") {
            Ok(t) => t,
            Err(_) => return,
        };
        assert_eq!(format!("{target}"), "linux_x64");
    }

    #[test]
    fn to_konanc_arg_returns_triple() {
        let target = match Target::from_str("macos_arm64") {
            Ok(t) => t,
            Err(_) => return,
        };
        assert_eq!(target.to_konanc_arg(), "macos_arm64");
    }

    #[test]
    fn is_host_matches_host_target() {
        let host = match host_target() {
            Ok(t) => t,
            Err(_) => return,
        };
        let result = match host.is_host() {
            Ok(v) => v,
            Err(_) => return,
        };
        assert!(result, "is_host() should return true for host_target()");
    }

    #[test]
    fn is_host_returns_false_for_non_host() {
        // Pick a target that definitely isn't the host on at least one platform
        let non_host_name = if cfg!(target_os = "linux") {
            "macos_arm64"
        } else {
            "linux_x64"
        };
        let target = match Target::from_str(non_host_name) {
            Ok(t) => t,
            Err(_) => return,
        };
        let result = match target.is_host() {
            Ok(v) => v,
            Err(_) => return,
        };
        assert!(
            !result,
            "is_host() should return false for `{non_host_name}` on this platform"
        );
    }

    #[test]
    fn target_equality() {
        let a = match Target::from_str("linux_x64") {
            Ok(t) => t,
            Err(_) => return,
        };
        let b = match Target::from_str("linux_x64") {
            Ok(t) => t,
            Err(_) => return,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn target_inequality() {
        let a = match Target::from_str("linux_x64") {
            Ok(t) => t,
            Err(_) => return,
        };
        let b = match Target::from_str("macos_arm64") {
            Ok(t) => t,
            Err(_) => return,
        };
        assert_ne!(a, b);
    }
}
