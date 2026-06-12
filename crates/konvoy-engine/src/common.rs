//! Small shared helpers used across multiple engine modules.

use crate::error::EngineError;

/// What to do about a single managed artifact under the orthogonal
/// `--locked` / `--offline` flags. Returned by [`gate_artifact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactGate {
    /// The lockfile lacks a pin for this artifact, so under `--locked` it would
    /// have to change to record one. Each path maps this to
    /// [`EngineError::LockfileUpdateRequired`](crate::error::EngineError::LockfileUpdateRequired).
    LockfileDrift,
    /// The artifact is absent locally and `--offline` forbids fetching it. Each
    /// path maps this to its own `…Offline` error.
    OfflineUnavailable,
    /// Fetch/verify the artifact normally: download it if absent, or re-verify
    /// the cached copy against its pin (no network on a cache hit).
    Proceed,
}

impl ArtifactGate {
    /// Map the gate decision to a `Result`, supplying the path-specific
    /// `--offline` error lazily. `LockfileDrift` always maps to
    /// [`EngineError::LockfileUpdateRequired`]; `Proceed` is `Ok(())`.
    ///
    /// Centralizes the per-path match so every managed-artifact site reads
    /// `gate_artifact(...).into_result(|| <path>Offline { .. })?`.
    pub(crate) fn into_result(
        self,
        offline_error: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        match self {
            Self::LockfileDrift => Err(EngineError::LockfileUpdateRequired),
            Self::OfflineUnavailable => Err(offline_error()),
            Self::Proceed => Ok(()),
        }
    }
}

/// Decide what to do about a single managed artifact under Konvoy's two
/// orthogonal reproducibility flags (Cargo's model — `--frozen` == both):
///
/// - `--locked` = reproducible install. Downloading a *pinned* artifact (and
///   verifying its SHA) is allowed; the only failure is lockfile drift, i.e. a
///   missing pin (`has_pin == false`).
/// - `--offline` = no network at all. The artifact must already be present
///   locally (`is_present == true`); otherwise it is a hard error.
///
/// `LockfileDrift` is checked FIRST: under `--frozen` (both flags set) a missing
/// pin is the actionable root cause, so it takes precedence over the offline
/// error — fixing the lockfile is what the user must do regardless.
pub(crate) fn gate_artifact(
    has_pin: bool,
    is_present: bool,
    locked: bool,
    offline: bool,
) -> ArtifactGate {
    if locked && !has_pin {
        ArtifactGate::LockfileDrift
    } else if offline && !is_present {
        ArtifactGate::OfflineUnavailable
    } else {
        ArtifactGate::Proceed
    }
}

/// Split a `groupId:artifactId` string into its two parts.
///
/// # Errors
///
/// Returns an error if the string does not contain exactly one colon.
pub(crate) fn split_maven_coordinate(maven: &str) -> Result<(&str, &str), EngineError> {
    maven
        .split_once(':')
        .ok_or_else(|| EngineError::InvalidMavenCoordinate {
            coordinate: maven.to_owned(),
            reason: "expected `groupId:artifactId`".to_owned(),
        })
}

/// Truncate a hex hash string to `len` characters for display.
///
/// Returns the full hash if it is shorter than `len`.
pub(crate) fn truncate_hash(hash: &str, len: usize) -> &str {
    hash.get(..len).unwrap_or(hash)
}

/// Current UTC timestamp formatted as `"{seconds}s-since-epoch"`.
///
/// Stored in build metadata; the suffix is purely for human reading.
pub(crate) fn now_epoch_secs() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}s-since-epoch", duration.as_secs())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ── gate_artifact: the unified --locked / --offline decision matrix ──
    //
    // Every managed-artifact path (toolchain, plugins, detekt JAR + JRE) routes
    // its fetch decision through `gate_artifact`, so this table is the single
    // source of truth for how the two flags combine.

    #[test]
    fn gate_default_mode_always_proceeds() {
        // Neither flag: download-if-absent, verify-if-present — always Proceed,
        // regardless of pin/presence.
        for has_pin in [false, true] {
            for is_present in [false, true] {
                assert_eq!(
                    gate_artifact(has_pin, is_present, false, false),
                    ArtifactGate::Proceed,
                    "has_pin={has_pin} is_present={is_present}"
                );
            }
        }
    }

    #[test]
    fn gate_locked_missing_pin_is_drift_regardless_of_presence() {
        // --locked with no pin is drift whether or not the artifact is cached:
        // the lockfile would have to change to record the pin.
        assert_eq!(
            gate_artifact(false, false, true, false),
            ArtifactGate::LockfileDrift
        );
        assert_eq!(
            gate_artifact(false, true, true, false),
            ArtifactGate::LockfileDrift
        );
    }

    #[test]
    fn gate_locked_with_pin_proceeds_even_when_absent() {
        // The whole point of the unification: --locked downloads a pinned-but-
        // absent artifact instead of erroring.
        assert_eq!(
            gate_artifact(true, false, true, false),
            ArtifactGate::Proceed
        );
        assert_eq!(
            gate_artifact(true, true, true, false),
            ArtifactGate::Proceed
        );
    }

    #[test]
    fn gate_offline_absent_is_unavailable() {
        // --offline with the artifact absent is a hard error, pin or not.
        assert_eq!(
            gate_artifact(true, false, false, true),
            ArtifactGate::OfflineUnavailable
        );
    }

    #[test]
    fn gate_offline_present_proceeds() {
        // --offline with the artifact present proceeds (re-verify, no network).
        assert_eq!(
            gate_artifact(true, true, false, true),
            ArtifactGate::Proceed
        );
        assert_eq!(
            gate_artifact(false, true, false, true),
            ArtifactGate::Proceed
        );
    }

    #[test]
    fn gate_frozen_drift_wins_over_offline() {
        // --frozen (both flags) with a missing pin AND an absent artifact: drift
        // is the actionable root cause, so it takes precedence over the offline
        // error.
        assert_eq!(
            gate_artifact(false, false, true, true),
            ArtifactGate::LockfileDrift
        );
    }

    #[test]
    fn gate_frozen_pinned_absent_is_offline() {
        // --frozen with a pin present but the artifact absent: no drift, so the
        // offline branch fires.
        assert_eq!(
            gate_artifact(true, false, true, true),
            ArtifactGate::OfflineUnavailable
        );
    }

    #[test]
    fn gate_frozen_pinned_present_proceeds() {
        assert_eq!(gate_artifact(true, true, true, true), ArtifactGate::Proceed);
    }

    #[test]
    fn into_result_maps_all_variants() {
        // Proceed -> Ok; LockfileDrift -> LockfileUpdateRequired (always);
        // OfflineUnavailable -> the supplied path-specific error.
        assert!(ArtifactGate::Proceed
            .into_result(|| EngineError::LintNotConfigured)
            .is_ok());
        assert!(matches!(
            ArtifactGate::LockfileDrift.into_result(|| EngineError::LintNotConfigured),
            Err(EngineError::LockfileUpdateRequired)
        ));
        assert!(matches!(
            ArtifactGate::OfflineUnavailable.into_result(|| EngineError::LintNotConfigured),
            Err(EngineError::LintNotConfigured)
        ));
    }

    #[test]
    fn gate_unified_matrix_is_path_independent() {
        // The three rows every managed-artifact path (toolchain, plugins, detekt
        // JAR + JRE) must obey identically — they all flow through this one
        // function, so the table holds regardless of which artifact it is:
        //
        //   (locked, pin missing)   -> LockfileDrift      (-> LockfileUpdateRequired)
        //   (offline, absent)       -> OfflineUnavailable (-> <path>Offline)
        //   (locked, pinned+present)-> Proceed            (-> Ok, no lockfile rewrite)
        assert_eq!(
            gate_artifact(/* has_pin */ false, /* present */ false, true, false),
            ArtifactGate::LockfileDrift,
            "locked + missing pin must be drift"
        );
        assert_eq!(
            gate_artifact(/* has_pin */ true, /* present */ false, false, true),
            ArtifactGate::OfflineUnavailable,
            "offline + absent must be unavailable"
        );
        assert_eq!(
            gate_artifact(/* has_pin */ true, /* present */ true, true, false),
            ArtifactGate::Proceed,
            "locked + pinned + present must proceed (and write nothing)"
        );
    }

    #[test]
    fn truncate_hash_short() {
        assert_eq!(
            truncate_hash("abcdef1234567890abcdef", 16),
            "abcdef1234567890"
        );
    }

    #[test]
    fn truncate_hash_shorter_than_limit() {
        assert_eq!(truncate_hash("abc", 16), "abc");
    }

    #[test]
    fn now_epoch_secs_format() {
        let ts = now_epoch_secs();
        assert!(
            ts.ends_with("s-since-epoch"),
            "expected format '<digits>s-since-epoch', got: {ts}"
        );
        let digits = ts.strip_suffix("s-since-epoch").unwrap();
        assert!(
            digits.parse::<u64>().is_ok(),
            "expected numeric prefix, got: {digits}"
        );
    }

    #[test]
    fn now_epoch_secs_is_reasonable() {
        let ts = now_epoch_secs();
        let secs: u64 = ts.strip_suffix("s-since-epoch").unwrap().parse().unwrap();
        // Should be after 2024-01-01 (1704067200) and before 2040-01-01 (2208988800).
        assert!(secs > 1_704_067_200, "timestamp too old: {secs}");
        assert!(secs < 2_208_988_800, "timestamp too far in future: {secs}");
    }
}
