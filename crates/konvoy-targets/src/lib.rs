#![forbid(unsafe_code)]
//! Host detection and target triple mapping for Konvoy.
//!
//! Maps Rust compile-time platform information to Kotlin/Native target names
//! and provides validation for user-supplied `--target` flags.

use std::fmt;
use std::str::FromStr;

/// A Kotlin/Native compilation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    LinuxX64,
    LinuxArm64,
    MacOsX64,
    MacOsArm64,
}

/// All known Kotlin/Native targets supported by Konvoy.
pub const KNOWN_TARGETS: &[Target] = &[
    Target::LinuxX64,
    Target::LinuxArm64,
    Target::MacOsX64,
    Target::MacOsArm64,
];

impl Target {
    /// Returns the string passed to `konanc -target` (e.g. `"linux_x64"`).
    pub fn to_konanc_arg(self) -> &'static str {
        match self {
            Target::LinuxX64 => "linux_x64",
            Target::LinuxArm64 => "linux_arm64",
            Target::MacOsX64 => "macos_x64",
            Target::MacOsArm64 => "macos_arm64",
        }
    }

    /// Returns the Maven artifact name suffix (underscores stripped).
    ///
    /// Kotlin/Native klibs published to Maven repositories use target names
    /// without underscores: `linux_x64` → `linuxx64`, `macos_arm64` → `macosarm64`.
    pub fn to_maven_suffix(self) -> &'static str {
        match self {
            Target::LinuxX64 => "linuxx64",
            Target::LinuxArm64 => "linuxarm64",
            Target::MacOsX64 => "macosx64",
            Target::MacOsArm64 => "macosarm64",
        }
    }

    /// Returns `true` if this target matches the current host platform.
    ///
    /// Returns `false` on unsupported hosts (where `host_target()` would error).
    pub fn is_host(self) -> bool {
        matches!(host_target(), Ok(host) if host == self)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_konanc_arg())
    }
}

impl FromStr for Target {
    type Err = TargetError;

    /// Parse and validate a user-supplied target string.
    ///
    /// # Errors
    /// Returns `TargetError::InvalidTarget` if the string is not a known Kotlin/Native target.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "linux_x64" => Ok(Target::LinuxX64),
            "linux_arm64" => Ok(Target::LinuxArm64),
            "macos_x64" => Ok(Target::MacOsX64),
            "macos_arm64" => Ok(Target::MacOsArm64),
            _ => Err(TargetError::InvalidTarget { name: s.to_owned() }),
        }
    }
}

impl serde::Serialize for Target {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.to_konanc_arg())
    }
}

impl<'de> serde::Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Detect the host target triple for Kotlin/Native.
///
/// # Errors
/// Returns `TargetError::UnsupportedHost` if the current OS/arch is not one of
/// the four Kotlin/Native host targets Konvoy supports.
pub fn host_target() -> Result<Target, TargetError> {
    target_for_host(std::env::consts::OS, std::env::consts::ARCH)
}

/// Internal helper: map a `(os, arch)` pair to a `Target`.
///
/// Split out from `host_target` so unit tests can exercise every match arm
/// without spawning subprocesses or cross-compiling. Production callers should
/// use `host_target()` which fills in the compile-time `OS` and `ARCH`.
fn target_for_host(os: &str, arch: &str) -> Result<Target, TargetError> {
    match (os, arch) {
        ("linux", "x86_64") => Ok(Target::LinuxX64),
        ("linux", "aarch64") => Ok(Target::LinuxArm64),
        ("macos", "x86_64") => Ok(Target::MacOsX64),
        ("macos", "aarch64") => Ok(Target::MacOsArm64),
        (os, arch) => Err(TargetError::UnsupportedHost {
            os: os.to_owned(),
            arch: arch.to_owned(),
        }),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    #[error("unsupported host: {os}/{arch} — Kotlin/Native does not support this platform")]
    UnsupportedHost { os: String, arch: String },

    #[error(
        "unknown target `{name}`, supported targets: host, linux_x64, linux_arm64, macos_x64, macos_arm64"
    )]
    InvalidTarget { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolve the host target for tests. Konvoy only supports the four
    /// Kotlin/Native host triples, so any CI/dev machine running this suite
    /// must already be one of them — `host_target()` succeeding is part of
    /// the project contract, not best-effort.
    fn require_host() -> Target {
        host_target().expect("konvoy only supports linux_x64/arm64 and macos_x64/arm64 hosts")
    }

    #[test]
    fn target_for_host_maps_every_supported_pair() {
        assert_eq!(
            target_for_host("linux", "x86_64").ok(),
            Some(Target::LinuxX64)
        );
        assert_eq!(
            target_for_host("linux", "aarch64").ok(),
            Some(Target::LinuxArm64)
        );
        assert_eq!(
            target_for_host("macos", "x86_64").ok(),
            Some(Target::MacOsX64)
        );
        assert_eq!(
            target_for_host("macos", "aarch64").ok(),
            Some(Target::MacOsArm64)
        );
    }

    #[test]
    fn target_for_host_rejects_unknown_os() {
        let err = target_for_host("windows", "x86_64").expect_err("windows is not supported");
        let msg = err.to_string();
        assert!(msg.contains("windows"));
        assert!(msg.contains("x86_64"));
    }

    #[test]
    fn target_for_host_rejects_unknown_arch_on_supported_os() {
        let err = target_for_host("linux", "riscv64").expect_err("riscv64 is not supported");
        let msg = err.to_string();
        assert!(msg.contains("linux"));
        assert!(msg.contains("riscv64"));
    }

    #[test]
    fn target_for_host_rejects_empty_strings() {
        assert!(target_for_host("", "").is_err());
        assert!(target_for_host("linux", "").is_err());
        assert!(target_for_host("", "x86_64").is_err());
    }

    #[test]
    fn host_target_returns_valid_known_target() {
        let target = require_host();
        assert!(KNOWN_TARGETS.contains(&target));
    }

    #[test]
    fn host_target_display_matches_konanc_arg() {
        let target = require_host();
        assert_eq!(target.to_string(), target.to_konanc_arg());
    }

    #[test]
    fn unsupported_host_error_displays_components() {
        // We can't actually swap out compile-time OS/ARCH, but exercising the
        // error constructor and Display keeps the variant from rotting.
        let err = TargetError::UnsupportedHost {
            os: "plan9".to_owned(),
            arch: "riscv64".to_owned(),
        };
        let msg = err.to_string();
        assert!(msg.contains("plan9"));
        assert!(msg.contains("riscv64"));
        assert!(msg.contains("Kotlin/Native"));
    }

    #[test]
    fn from_str_accepts_all_known_targets() {
        for &t in KNOWN_TARGETS {
            let parsed = Target::from_str(t.to_konanc_arg());
            assert_eq!(parsed.ok(), Some(t));
        }
    }

    #[test]
    fn from_str_rejects_invalid_target() {
        assert!(Target::from_str("windows_x64").is_err());
    }

    #[test]
    fn from_str_rejects_empty_string() {
        assert!(Target::from_str("").is_err());
    }

    #[test]
    fn invalid_target_error_lists_supported_targets() {
        let err = Target::from_str("bsd_x64").expect_err("bsd_x64 should not parse");
        let msg = err.to_string();
        for t in KNOWN_TARGETS {
            assert!(
                msg.contains(t.to_konanc_arg()),
                "error message should list `{t}`, got: {msg}"
            );
        }
    }

    #[test]
    fn invalid_target_error_mentions_host_alias() {
        let err = Target::from_str("bsd_x64").expect_err("bsd_x64 should not parse");
        assert!(err.to_string().contains("host"));
    }

    #[test]
    fn invalid_target_error_includes_user_supplied_name() {
        let err = Target::from_str("aix_power").expect_err("aix_power should not parse");
        assert!(
            err.to_string().contains("aix_power"),
            "error should echo the user-supplied name, got: {err}"
        );
    }

    #[test]
    fn invalid_target_error_debug_format_succeeds() {
        // Exercise the auto-derived Debug impl on `TargetError` so refactors
        // that swap variants don't accidentally drop the bound.
        let err = TargetError::InvalidTarget {
            name: "weird".to_owned(),
        };
        let dbg = format!("{err:?}");
        assert!(dbg.contains("InvalidTarget"));
        assert!(dbg.contains("weird"));
    }

    #[test]
    fn unsupported_host_error_debug_format_succeeds() {
        let err = TargetError::UnsupportedHost {
            os: "plan9".to_owned(),
            arch: "riscv64".to_owned(),
        };
        let dbg = format!("{err:?}");
        assert!(dbg.contains("UnsupportedHost"));
        assert!(dbg.contains("plan9"));
        assert!(dbg.contains("riscv64"));
    }

    #[test]
    fn unsupported_host_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TargetError>();
    }

    #[test]
    fn display_format_matches_konanc_arg() {
        let target = Target::LinuxX64;
        assert_eq!(format!("{target}"), "linux_x64");
    }

    #[test]
    fn to_konanc_arg_returns_triple() {
        assert_eq!(Target::MacOsArm64.to_konanc_arg(), "macos_arm64");
    }

    #[test]
    fn is_host_matches_host_target() {
        let host = require_host();
        assert!(host.is_host());
    }

    #[test]
    fn is_host_returns_false_for_non_host() {
        let host = require_host();
        // Pick any target that isn't the host so we exercise the `false` arm
        // without baking in a host-specific branch (which would leave one of
        // the alternative arms permanently uncovered on a fixed CI host).
        let non_host = KNOWN_TARGETS
            .iter()
            .copied()
            .find(|t| *t != host)
            .expect("KNOWN_TARGETS has more than one entry");
        assert!(!non_host.is_host());
    }

    #[test]
    fn target_equality() {
        let a: Target = "linux_x64".parse().expect("valid");
        let b: Target = "linux_x64".parse().expect("valid");
        assert_eq!(a, b);
    }

    #[test]
    fn target_inequality() {
        let a: Target = "linux_x64".parse().expect("valid");
        let b: Target = "macos_arm64".parse().expect("valid");
        assert_ne!(a, b);
    }

    #[test]
    fn to_maven_suffix_linux_x64() {
        assert_eq!(Target::LinuxX64.to_maven_suffix(), "linuxx64");
    }

    #[test]
    fn to_maven_suffix_macos_arm64() {
        assert_eq!(Target::MacOsArm64.to_maven_suffix(), "macosarm64");
    }

    #[test]
    fn to_maven_suffix_no_underscores() {
        for &t in KNOWN_TARGETS {
            let suffix = t.to_maven_suffix();
            assert!(
                !suffix.contains('_'),
                "to_maven_suffix() for `{t}` should not contain underscores, got `{suffix}`"
            );
        }
    }

    #[test]
    fn serde_serializes_as_konanc_arg() {
        #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
        struct Wrap {
            target: Target,
        }
        let s = toml::to_string(&Wrap {
            target: Target::LinuxX64,
        })
        .expect("serialize");
        assert!(
            s.contains("target = \"linux_x64\""),
            "expected wire `linux_x64`, got: {s}"
        );
        let back: Wrap = toml::from_str("target = \"macos_arm64\"").expect("deserialize");
        assert_eq!(back.target, Target::MacOsArm64);
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn arbitrary_target_never_panics(s in "\\PC*") {
                let result = Target::from_str(&s);
                prop_assert!(result.is_ok() || result.is_err());
            }

            #[test]
            fn known_targets_always_parse(idx in 0usize..4) {
                let t = KNOWN_TARGETS[idx];
                let parsed = Target::from_str(t.to_konanc_arg());
                prop_assert_eq!(parsed.ok(), Some(t));
            }

            #[test]
            fn maven_suffix_never_contains_underscore(idx in 0usize..4) {
                let t = KNOWN_TARGETS[idx];
                prop_assert!(!t.to_maven_suffix().contains('_'));
            }

            #[test]
            fn unknown_targets_always_error(s in "\\PC*") {
                let known: Vec<&'static str> =
                    KNOWN_TARGETS.iter().map(|t| t.to_konanc_arg()).collect();
                prop_assume!(!known.contains(&s.as_str()));
                let result = Target::from_str(&s);
                prop_assert!(result.is_err());
            }
        }
    }
}
