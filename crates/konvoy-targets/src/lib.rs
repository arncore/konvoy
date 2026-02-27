#![forbid(unsafe_code)]
//! Host detection and target triple mapping for Konvoy.
//!
//! Maps Rust compile-time platform information to Kotlin/Native target names
//! and provides validation for user-supplied `--target` flags.

use std::fmt;
use std::str::FromStr;

/// All known Kotlin/Native targets supported by Konvoy.
const KNOWN_TARGETS: &[&str] = &["linux_x64", "linux_arm64", "macos_x64", "macos_arm64"];

/// Returns the list of all known Kotlin/Native target names.
pub fn known_targets() -> &'static [&'static str] {
    KNOWN_TARGETS
}

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

    /// Returns the Maven artifact name suffix (underscores stripped).
    ///
    /// Kotlin/Native klibs published to Maven repositories use target names
    /// without underscores: `linux_x64` → `linuxx64`, `macos_arm64` → `macosarm64`.
    pub fn to_maven_suffix(&self) -> String {
        self.triple.replace('_', "")
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
        "unknown target `{name}`, supported targets: host, {}",
        KNOWN_TARGETS.join(", ")
    )]
    InvalidTarget { name: String },
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
    fn invalid_target_error_mentions_host_alias() {
        let err = Target::from_str("bsd_x64");
        let msg = match err {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"), // only in test code
        };
        assert!(
            msg.contains("host"),
            "error message should mention `host` alias, got: {msg}"
        );
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

    #[test]
    fn to_maven_suffix_linux_x64() {
        let target = Target::from_str("linux_x64").unwrap();
        assert_eq!(target.to_maven_suffix(), "linuxx64");
    }

    #[test]
    fn to_maven_suffix_macos_arm64() {
        let target = Target::from_str("macos_arm64").unwrap();
        assert_eq!(target.to_maven_suffix(), "macosarm64");
    }

    #[test]
    fn to_maven_suffix_no_underscores() {
        for &name in KNOWN_TARGETS {
            let target = Target::from_str(name).unwrap();
            let suffix = target.to_maven_suffix();
            assert!(
                !suffix.contains('_'),
                "to_maven_suffix() for `{name}` should not contain underscores, got `{suffix}`"
            );
        }
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            #[allow(clippy::unwrap_used)]
            fn arbitrary_target_never_panics(s in "\\PC*") {
                let result = Target::from_str(&s);
                prop_assert!(result.is_ok() || result.is_err());
            }

            #[test]
            #[allow(clippy::unwrap_used)]
            fn known_targets_always_parse(idx in 0usize..4) {
                let known = ["linux_x64", "linux_arm64", "macos_x64", "macos_arm64"];
                let name = known[idx];
                let target = Target::from_str(name);
                prop_assert!(target.is_ok(), "from_str rejected known target `{}`", name);
                let target = target.unwrap();
                prop_assert_eq!(target.to_string(), name);
            }

            #[test]
            #[allow(clippy::unwrap_used)]
            fn maven_suffix_never_contains_underscore(idx in 0usize..4) {
                let known = ["linux_x64", "linux_arm64", "macos_x64", "macos_arm64"];
                let name = known[idx];
                let target = Target::from_str(name).unwrap();
                let suffix = target.to_maven_suffix();
                prop_assert!(!suffix.contains('_'), "maven suffix `{}` contains underscore", suffix);
            }

            #[test]
            #[allow(clippy::unwrap_used)]
            fn unknown_targets_always_error(s in "\\PC*") {
                let known = ["linux_x64", "linux_arm64", "macos_x64", "macos_arm64"];
                prop_assume!(!known.contains(&s.as_str()));
                let result = Target::from_str(&s);
                prop_assert!(result.is_err(), "from_str accepted unknown target `{}`", s);
            }
        }
    }
}
