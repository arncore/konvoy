//! Small shared helpers used across multiple engine modules.

use crate::error::EngineError;

/// Per-command resolver for managed artifacts.
///
/// This keeps artifact fetching and the shared network client behind one
/// command-scoped construct. Callers ask it to resolve artifacts; they do not
/// need to branch on the network mode directly.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactResolver<'a> {
    net: &'a konvoy_util::net::NetworkClient,
    lockfiles: LockfileManager,
}

impl<'a> ArtifactResolver<'a> {
    /// Create an artifact resolver for one command invocation.
    #[must_use]
    pub const fn new(net: &'a konvoy_util::net::NetworkClient, lockfiles: LockfileManager) -> Self {
        Self { net, lockfiles }
    }

    /// Require an artifact to be locally present when the command is offline.
    fn require_available(
        self,
        is_present: bool,
        offline_error: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        if self.net.is_offline() && !is_present {
            Err(offline_error())
        } else {
            Ok(())
        }
    }

    /// Resolve whether a managed artifact may be used or fetched.
    ///
    /// Lockfile drift is checked first so `--locked --offline` reports the
    /// lockfile problem before reporting a cache miss.
    pub(crate) fn resolve_artifact(
        self,
        has_pin: bool,
        is_present: bool,
        offline_error: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        self.lockfiles.require_artifact_pin(has_pin)?;
        self.require_available(is_present, offline_error)
    }

    /// Run a lockfile-updating resolution path only when policy and network
    /// mode allow it.
    pub(crate) fn resolve_lockfile_update<T>(
        self,
        offline_error: impl FnOnce() -> EngineError,
        resolve: impl FnOnce() -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        self.lockfiles.require_update_allowed()?;
        self.require_available(false, offline_error)?;
        resolve()
    }

    /// Run a lockfile staleness check only when locked policy requires it.
    pub(crate) fn verify_current_lockfile(
        self,
        check: impl FnOnce() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        self.lockfiles.verify_current_lockfile(check)
    }

    /// Reject a lockfile-changing condition when locked policy forbids writes.
    pub(crate) fn reject_lockfile_change(
        self,
        err: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        self.lockfiles.reject_if_locked(err)
    }

    /// Return the candidate lockfile content that should feed cache keys.
    pub(crate) fn effective_lockfile(
        self,
        current: &konvoy_config::lockfile::Lockfile,
        unlocked: impl FnOnce() -> konvoy_config::lockfile::Lockfile,
    ) -> konvoy_config::lockfile::Lockfile {
        self.lockfiles.effective_lockfile(current, unlocked)
    }

    /// Write a changed lockfile candidate, or reject the write when locked.
    pub(crate) fn write_updated_lockfile(
        self,
        current: &konvoy_config::lockfile::Lockfile,
        updated: konvoy_config::lockfile::Lockfile,
        lockfile_path: &std::path::Path,
    ) -> Result<(), EngineError> {
        self.lockfiles
            .write_updated_lockfile(current, updated, lockfile_path)
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

    /// Require a lockfile pin when locked policy forbids lockfile updates.
    fn require_artifact_pin(self, has_pin: bool) -> Result<(), EngineError> {
        if self.locked && !has_pin {
            Err(EngineError::LockfileUpdateRequired)
        } else {
            Ok(())
        }
    }

    /// Require that the command may update the lockfile.
    fn require_update_allowed(self) -> Result<(), EngineError> {
        if self.locked {
            Err(EngineError::LockfileUpdateRequired)
        } else {
            Ok(())
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

    #[test]
    fn managed_artifact_reports_lockfile_drift_before_offline() {
        let net = konvoy_util::net::NetworkClient::new(true);
        let resolver = ArtifactResolver::new(&net, LockfileManager::new(true));

        let result = resolver.resolve_artifact(false, false, || EngineError::LintNotConfigured);

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn managed_artifact_maps_offline_unavailable() {
        let net = konvoy_util::net::NetworkClient::new(true);
        let resolver = ArtifactResolver::new(&net, LockfileManager::new(false));

        let result = resolver.resolve_artifact(true, false, || EngineError::LintNotConfigured);

        assert!(matches!(result, Err(EngineError::LintNotConfigured)));
    }

    #[test]
    fn managed_artifact_allows_locked_pinned_absent_when_online() {
        let net = konvoy_util::net::NetworkClient::new(false);
        let resolver = ArtifactResolver::new(&net, LockfileManager::new(true));

        let result = resolver.resolve_artifact(true, false, || EngineError::LintNotConfigured);

        assert!(result.is_ok());
    }

    #[test]
    fn lockfile_update_resolves_only_when_allowed_and_online() {
        let online = konvoy_util::net::NetworkClient::new(false);
        let resolver = ArtifactResolver::new(&online, LockfileManager::new(false));

        let result = resolver.resolve_lockfile_update(
            || EngineError::LintNotConfigured,
            || Ok::<_, EngineError>("resolved"),
        );

        assert_eq!(result.unwrap(), "resolved");

        let locked = ArtifactResolver::new(&online, LockfileManager::new(true))
            .resolve_lockfile_update(
                || EngineError::LintNotConfigured,
                || Ok::<_, EngineError>("resolved"),
            );
        assert!(matches!(locked, Err(EngineError::LockfileUpdateRequired)));
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
