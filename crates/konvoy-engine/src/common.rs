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

/// Per-command resolver for managed artifacts.
///
/// This keeps artifact fetching and the shared network client behind one
/// command-scoped construct. Callers ask it to resolve artifacts; they do not
/// need to branch on the network mode directly.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactResolver<'a> {
    net: &'a konvoy_util::net::NetworkClient,
}

impl<'a> ArtifactResolver<'a> {
    /// Create an artifact resolver for one command invocation.
    #[must_use]
    pub const fn new(net: &'a konvoy_util::net::NetworkClient) -> Self {
        Self { net }
    }

    /// Resolve whether an already-pinned artifact may be used or fetched.
    fn resolve_available_artifact(
        self,
        is_present: bool,
        offline_error: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        gate_artifact(true, is_present, false, self.net.is_offline()).into_result(offline_error)
    }

    /// Resolve and, if necessary, install the managed Kotlin/Native toolchain.
    pub(crate) fn resolve_konanc(
        self,
        version: &str,
    ) -> Result<konvoy_konanc::detect::ResolvedKonanc, konvoy_konanc::error::KonancError> {
        konvoy_konanc::detect::resolve_konanc(version, self.net)
    }

    /// Install the managed Kotlin/Native toolchain.
    pub(crate) fn install_toolchain(
        self,
        version: &str,
    ) -> Result<konvoy_konanc::toolchain::InstallResult, konvoy_konanc::error::KonancError> {
        konvoy_konanc::toolchain::install(version, self.net)
    }

    /// Ensure a managed tool artifact exists and is verified.
    pub(crate) fn ensure_managed_tool(
        self,
        tool: &crate::managed_tool::ManagedToolSpec,
        expected_sha256: Option<&str>,
    ) -> Result<(std::path::PathBuf, String), konvoy_util::error::UtilError> {
        tool.ensure(expected_sha256, self.net)
    }

    /// Fetch or verify a managed artifact.
    pub(crate) fn fetch_artifact(
        self,
        url: &str,
        dest: &std::path::Path,
        expected_sha256: Option<&str>,
        label: &str,
        bar: Option<&konvoy_util::progress::DownloadBar>,
    ) -> Result<konvoy_util::artifact::ArtifactResult, konvoy_util::error::UtilError> {
        konvoy_util::progress::fetch(self.net, url, dest, expected_sha256, label, bar)
    }

    /// Fetch artifact metadata for Maven dependency resolution.
    pub(crate) fn fetch_artifact_metadata(
        self,
        group_id: &str,
        artifact_id: &str,
        version: &str,
        maven_suffix: &str,
    ) -> Result<konvoy_util::metadata::ArtifactMetadata, konvoy_util::error::UtilError> {
        konvoy_util::metadata::fetch_artifact_metadata(
            self.net,
            group_id,
            artifact_id,
            version,
            maven_suffix,
        )
    }
}

/// Per-command manager for lockfile policy.
///
/// This keeps `--locked` behind a command-scoped construct. Callers ask the
/// manager whether a lockfile-affecting action may proceed; they do not branch
/// on the flag directly.
#[derive(Debug, Clone, Copy)]
pub struct LockfileManager {
    locked: bool,
}

impl LockfileManager {
    /// Create a lockfile manager for one command invocation.
    #[must_use]
    pub const fn new(locked: bool) -> Self {
        Self { locked }
    }

    /// Resolve whether an artifact may be used or fetched without changing the
    /// lockfile.
    pub(crate) fn resolve_artifact(
        self,
        artifacts: ArtifactResolver<'_>,
        has_pin: bool,
        is_present: bool,
        offline_error: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        if self.locked && !has_pin {
            Err(EngineError::LockfileUpdateRequired)
        } else {
            artifacts.resolve_available_artifact(is_present, offline_error)
        }
    }

    /// Resolve a lockfile drift path: fail under locked/offline policy, or run
    /// the supplied resolver when the command is allowed to update/fetch.
    pub(crate) fn resolve_lockfile_drift<T>(
        self,
        artifacts: ArtifactResolver<'_>,
        offline_error: impl FnOnce() -> EngineError,
        resolve: impl FnOnce() -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        if self.locked {
            Err(EngineError::LockfileUpdateRequired)
        } else {
            artifacts.resolve_available_artifact(false, offline_error)?;
            resolve()
        }
    }

    /// Run a lockfile staleness check only when locked policy requires it.
    pub(crate) fn verify_current_lockfile(
        self,
        check: impl FnOnce() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        if self.locked {
            check()
        } else {
            Ok(())
        }
    }

    /// Reject a lockfile-changing condition when locked policy forbids writes.
    pub(crate) fn reject_if_locked(
        self,
        err: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        if self.locked {
            Err(err())
        } else {
            Ok(())
        }
    }

    /// Return the candidate lockfile content that should feed cache keys.
    pub(crate) fn effective_lockfile(
        self,
        current: &konvoy_config::lockfile::Lockfile,
        unlocked: impl FnOnce() -> konvoy_config::lockfile::Lockfile,
    ) -> konvoy_config::lockfile::Lockfile {
        if self.locked {
            current.clone()
        } else {
            unlocked()
        }
    }

    /// Write a changed lockfile candidate, or reject the write when locked.
    pub(crate) fn write_updated_lockfile(
        self,
        current: &konvoy_config::lockfile::Lockfile,
        updated: konvoy_config::lockfile::Lockfile,
        lockfile_path: &std::path::Path,
    ) -> Result<(), EngineError> {
        if updated == *current {
            return Ok(());
        }
        if self.locked {
            return Err(EngineError::LockfileUpdateRequired);
        }
        updated.write_to(lockfile_path)?;
        Ok(())
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
    fn lockfile_manager_gates_lockfile_drift_before_offline() {
        let net = konvoy_util::net::NetworkClient::new(true);
        let resolver = ArtifactResolver::new(&net);
        let lockfiles = LockfileManager::new(true);

        let result =
            lockfiles.resolve_artifact(resolver, false, false, || EngineError::LintNotConfigured);

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn lockfile_manager_maps_offline_unavailable() {
        let net = konvoy_util::net::NetworkClient::new(true);
        let resolver = ArtifactResolver::new(&net);
        let lockfiles = LockfileManager::new(false);

        let result =
            lockfiles.resolve_artifact(resolver, true, false, || EngineError::LintNotConfigured);

        assert!(matches!(result, Err(EngineError::LintNotConfigured)));
    }

    #[test]
    fn lockfile_manager_resolves_lockfile_drift_only_when_allowed() {
        let online = konvoy_util::net::NetworkClient::new(false);
        let resolver = ArtifactResolver::new(&online);
        let lockfiles = LockfileManager::new(false);

        let result = lockfiles.resolve_lockfile_drift(
            resolver,
            || EngineError::LintNotConfigured,
            || Ok::<_, EngineError>("resolved"),
        );

        assert_eq!(result.unwrap(), "resolved");
    }

    #[test]
    fn lockfile_manager_verifies_current_lockfile_only_when_locked() {
        let unlocked = LockfileManager::new(false);
        let locked = LockfileManager::new(true);

        assert!(unlocked
            .verify_current_lockfile(|| Err(EngineError::LintNotConfigured))
            .is_ok());
        assert!(matches!(
            locked.verify_current_lockfile(|| Err(EngineError::LintNotConfigured)),
            Err(EngineError::LintNotConfigured)
        ));
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
