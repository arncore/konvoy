//! Small shared helpers used across multiple engine modules.

use crate::error::EngineError;
use konvoy_config::{lockfile::Lockfile, Manifest};

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
    /// lockfile problem before reporting a cache miss. `has_pin` is a closure so
    /// that any cost of computing it (e.g. stat-ing the install location) is
    /// skipped entirely when the command is not `--locked`.
    fn resolve_artifact(
        self,
        has_pin: impl FnOnce() -> Result<bool, EngineError>,
        is_present: bool,
        offline_error: impl FnOnce() -> EngineError,
    ) -> Result<(), EngineError> {
        self.lockfiles.require_artifact_pin(has_pin)?;
        self.require_available(is_present, offline_error)
    }

    /// Resolve and, if necessary, install the managed Kotlin/Native toolchain.
    pub(crate) fn resolve_toolchain(
        self,
        version: &str,
        lockfile: &Lockfile,
    ) -> Result<konvoy_konanc::detect::ResolvedKonanc, EngineError> {
        let is_present = konvoy_konanc::toolchain::is_installed(version)?;
        // `has_pin` stats the install location; passed lazily so the probe only
        // runs under --locked (the offline gate uses `is_present`, not the pin).
        self.resolve_artifact(
            || has_required_toolchain_artifact_pins(lockfile, version),
            is_present,
            || EngineError::ToolchainOffline {
                version: version.to_owned(),
            },
        )?;
        Ok(konvoy_konanc::detect::resolve_konanc(version, self.net)?)
    }

    /// Resolve the detekt CLI JAR.
    ///
    /// Returns `Some(hash)` — the freshly-verified hash to persist into the
    /// lockfile — when the JAR was not already pinned, or `None` when the
    /// lockfile already pins it (nothing to write).
    pub(crate) fn resolve_detekt_jar(
        self,
        version: &str,
        lockfile: &Lockfile,
    ) -> Result<Option<String>, EngineError> {
        let expected_sha256 = lockfile.toolchain.as_ref().and_then(|tc| {
            (tc.detekt_version.as_deref() == Some(version))
                .then_some(tc.detekt_jar_sha256.as_deref())
                .flatten()
                // An empty string is not a pin: download-and-record rather than
                // verify the JAR against an empty hash.
                .filter(|s| !s.is_empty())
        });
        let was_pinned = expected_sha256.is_some();
        let is_present = crate::detekt::is_installed(version)?;
        self.resolve_artifact(
            || Ok(was_pinned),
            is_present,
            || EngineError::DetektJarOffline {
                version: version.to_owned(),
            },
        )?;
        let (_, actual_sha256) = crate::detekt::ensure_detekt(version, expected_sha256, self)?;
        Ok((!was_pinned).then_some(actual_sha256))
    }

    /// Resolve the JRE used to run detekt.
    pub(crate) fn resolve_detekt_jre(
        self,
        kotlin_version: &str,
        lockfile: &Lockfile,
    ) -> Result<std::path::PathBuf, EngineError> {
        if !konvoy_konanc::toolchain::is_installed(kotlin_version)? {
            self.resolve_artifact(
                || has_required_toolchain_artifact_pins(lockfile, kotlin_version),
                false,
                || EngineError::DetektJreOffline {
                    version: kotlin_version.to_owned(),
                },
            )?;
            eprintln!("    Installing Kotlin/Native {kotlin_version} (for JRE)...");
            konvoy_konanc::toolchain::install(kotlin_version, self.net)?;
        }

        let jre_home = konvoy_konanc::toolchain::jre_home_path(kotlin_version)?;
        if !jre_home.join("bin").join("java").exists() {
            return Err(EngineError::DetektNoJre);
        }
        Ok(jre_home)
    }

    /// Resolve a code-generation tool (e.g. the Fabrikt JAR), returning its path
    /// and freshly-verified SHA-256.
    ///
    /// Mirrors [`resolve_detekt_jar`](Self::resolve_detekt_jar): gate on the
    /// `--locked` pin and `--offline` presence, then fetch-or-re-verify the
    /// artifact through the shared network client.
    pub(crate) fn resolve_codegen_tool(
        self,
        tool: &crate::managed_tool::ManagedToolSpec,
        expected_sha256: Option<&str>,
    ) -> Result<(std::path::PathBuf, String), EngineError> {
        let is_present = tool.is_installed().map_err(EngineError::from)?;
        self.resolve_artifact(
            || Ok(expected_sha256.is_some()),
            is_present,
            || EngineError::CodegenToolOffline {
                name: tool.id().to_owned(),
                version: tool.version().to_owned(),
            },
        )?;
        let version = tool.version().to_owned();
        self.ensure_managed_tool(tool, expected_sha256)
            .map_err(|e| crate::codegen::map_download_err(tool.id(), &version, e))
    }

    /// Resolve a compiler plugin artifact and return its verified artifact data.
    pub(crate) fn resolve_plugin_artifact(
        self,
        artifact: &crate::plugin::ResolvedPluginArtifact,
        lockfile: &Lockfile,
        bar: Option<&konvoy_util::progress::DownloadBar>,
    ) -> Result<crate::plugin::PluginArtifactResult, EngineError> {
        let expected_sha256 = crate::plugin::find_artifact_lockfile_hash(lockfile, artifact);
        self.resolve_artifact(
            || Ok(expected_sha256.is_some()),
            artifact.cache_path.exists(),
            || EngineError::PluginOffline {
                name: artifact.plugin_name.clone(),
            },
        )?;
        let util_result = self
            .fetch_artifact(
                &artifact.url,
                &artifact.cache_path,
                expected_sha256,
                &artifact.plugin_name,
                bar,
            )
            .map_err(|e| crate::plugin::map_download_err(&artifact.plugin_name, e))?;

        Ok(crate::plugin::PluginArtifactResult {
            plugin_name: artifact.plugin_name.clone(),
            path: util_result.path,
            sha256: util_result.sha256,
            url: artifact.url.clone(),
            freshly_downloaded: util_result.freshly_downloaded,
            maven: artifact.maven_coord.group_artifact(),
            version: artifact.maven_coord.version.clone(),
        })
    }

    /// Resolve plugin artifact cache state before scheduling downloads.
    pub(crate) fn prepare_plugin_artifacts(
        self,
        artifacts: &[crate::plugin::ResolvedPluginArtifact],
        lockfile: &Lockfile,
    ) -> Result<Vec<bool>, EngineError> {
        let present: Vec<bool> = artifacts.iter().map(|a| a.cache_path.exists()).collect();
        for (artifact, is_present) in artifacts.iter().zip(present.iter().copied()) {
            self.resolve_artifact(
                || Ok(crate::plugin::find_artifact_lockfile_hash(lockfile, artifact).is_some()),
                is_present,
                || EngineError::PluginOffline {
                    name: artifact.plugin_name.clone(),
                },
            )?;
        }
        Ok(present)
    }

    /// Resolve a Maven dependency klib and return a cache-key-ready library input.
    pub(crate) fn resolve_maven_klib(
        self,
        name: &str,
        url: &str,
        dest: &std::path::Path,
        expected_sha256: &str,
        bar: Option<&konvoy_util::progress::DownloadBar>,
    ) -> Result<crate::build::LibraryInput, EngineError> {
        self.resolve_artifact(
            || Ok(true),
            dest.exists(),
            || EngineError::LibraryOffline {
                name: name.to_owned(),
            },
        )?;
        let result = self
            .fetch_artifact(url, dest, Some(expected_sha256), name, bar)
            .map_err(|e| EngineError::LibraryDownloadFailed {
                name: name.to_owned(),
                url: url.to_owned(),
                message: e.to_string(),
            })?;
        Ok(crate::build::LibraryInput::with_hash(
            result.path,
            result.sha256,
        ))
    }

    /// Resolve missing Maven dependency state by running update when policy and
    /// network mode allow it.
    pub(crate) fn resolve_missing_maven_dependencies(
        self,
        project_root: &std::path::Path,
        manifest: &Manifest,
        dep_graph: &crate::resolve::ResolvedGraph,
        lockfile_path: &std::path::Path,
        name: String,
    ) -> Result<Lockfile, EngineError> {
        self.lockfiles.require_update_allowed()?;
        self.require_available(false, || EngineError::MissingLockfileEntry { name })?;
        eprintln!("  Maven dependencies not resolved - running update automatically...");
        // Reuse the already-resolved graph so we don't re-walk + re-hash every
        // path-dep's source tree a second time on this cold build.
        crate::update::update_with_graph(project_root, manifest, dep_graph, self)?;
        Ok(Lockfile::from_path(lockfile_path)?)
    }

    /// Require the manifest's managed artifacts to be resolvable under the
    /// command's policy (root-only — used by `lint`, which does not build the
    /// dependency graph).
    pub(crate) fn require_manifest_artifacts_resolvable(
        self,
        manifest: &Manifest,
        lockfile: &Lockfile,
    ) -> Result<(), EngineError> {
        self.lockfiles
            .verify_current_lockfile(|| crate::build::check_lockfile_staleness(manifest, lockfile))
    }

    /// Require the build graph's managed artifacts to be resolvable under the
    /// command's policy: the root's full staleness check plus, for every
    /// path-dependency, that its Maven deps are pinned in the (shared) root lock.
    pub(crate) fn require_graph_artifacts_resolvable<'m>(
        self,
        manifest: &Manifest,
        dep_manifests: impl IntoIterator<Item = &'m Manifest>,
        lockfile: &Lockfile,
    ) -> Result<(), EngineError> {
        self.lockfiles.verify_current_lockfile(|| {
            crate::build::check_graph_lockfile_staleness(manifest, dep_manifests, lockfile)
        })
    }

    /// Resolve a changed path dependency source under the command's policy.
    pub(crate) fn resolve_changed_dependency_source(
        self,
        name: &str,
        expected: &str,
        actual: &str,
    ) -> Result<(), EngineError> {
        self.lockfiles
            .reject_if_locked(|| EngineError::DependencyHashMismatch {
                name: name.to_owned(),
                expected: expected.to_owned(),
                actual: actual.to_owned(),
            })
    }

    /// Return the resolved artifact state that should feed cache keys.
    pub(crate) fn cache_key_artifact_state(
        self,
        current: &Lockfile,
        unlocked: impl FnOnce() -> Lockfile,
    ) -> Lockfile {
        self.lockfiles.effective_lockfile(current, unlocked)
    }

    /// Persist resolved artifact state, or fail when policy forbids changes.
    pub(crate) fn persist_resolved_artifacts(
        self,
        current: &Lockfile,
        updated: &Lockfile,
        lockfile_path: &std::path::Path,
    ) -> Result<(), EngineError> {
        self.lockfiles
            .write_updated_lockfile(current, updated, lockfile_path)
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

fn has_required_toolchain_artifact_pins(
    lockfile: &Lockfile,
    kotlin_version: &str,
) -> Result<bool, EngineError> {
    let Some(tc) = lockfile
        .toolchain
        .as_ref()
        .filter(|tc| tc.konanc_version == kotlin_version)
    else {
        return Ok(false);
    };

    let konanc_missing = !konvoy_konanc::toolchain::managed_konanc_path(kotlin_version)?.exists();
    let jre_missing = !konvoy_konanc::toolchain::jre_dir(kotlin_version)?.exists();

    let konanc_pinned = !konanc_missing
        || tc
            .konanc_tarball_sha256
            .as_deref()
            .is_some_and(|s| !s.is_empty());
    let jre_pinned = !jre_missing
        || tc
            .jre_tarball_sha256
            .as_deref()
            .is_some_and(|s| !s.is_empty());

    Ok(konanc_pinned && jre_pinned)
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
    ///
    /// `has_pin` is evaluated only under `--locked`, so callers can pass a
    /// closure whose cost (e.g. stat-ing an install location) is skipped
    /// entirely when the command is unlocked.
    fn require_artifact_pin(
        self,
        has_pin: impl FnOnce() -> Result<bool, EngineError>,
    ) -> Result<(), EngineError> {
        if self.locked && !has_pin()? {
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
    fn verify_current_lockfile(
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
    fn reject_if_locked(self, err: impl FnOnce() -> EngineError) -> Result<(), EngineError> {
        if self.locked {
            Err(err())
        } else {
            Ok(())
        }
    }

    /// Return the candidate lockfile content that should feed cache keys.
    fn effective_lockfile(
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
    fn write_updated_lockfile(
        self,
        current: &konvoy_config::lockfile::Lockfile,
        updated: &konvoy_config::lockfile::Lockfile,
        lockfile_path: &std::path::Path,
    ) -> Result<(), EngineError> {
        if updated == current {
            return Ok(());
        }
        if self.locked {
            return Err(EngineError::LockfileUpdateRequired);
        }
        updated.write_to(lockfile_path)?;
        Ok(())
    }
}

/// Test-only resolver constructor: leaks a `NetworkClient` (fine for tests) so
/// the borrowing `ArtifactResolver` can be returned directly, instead of every
/// test site repeating the net + manager + resolver wiring.
#[cfg(test)]
pub(crate) fn test_resolver(offline: bool, locked: bool) -> ArtifactResolver<'static> {
    let net = Box::leak(Box::new(konvoy_util::net::NetworkClient::new(offline)));
    ArtifactResolver::new(net, LockfileManager::new(locked))
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
    use konvoy_config::lockfile::{PluginLock, ToolchainLock};
    use konvoy_util::maven::MavenCoordinate;
    use std::cell::Cell;
    use std::path::PathBuf;

    fn with_resolver<T>(
        offline: bool,
        locked: bool,
        f: impl FnOnce(ArtifactResolver<'_>) -> T,
    ) -> T {
        let net = konvoy_util::net::NetworkClient::new(offline);
        f(ArtifactResolver::new(&net, LockfileManager::new(locked)))
    }

    fn plugin_artifact(name: &str, cache_path: PathBuf) -> crate::plugin::ResolvedPluginArtifact {
        crate::plugin::ResolvedPluginArtifact {
            plugin_name: name.to_owned(),
            maven_coord: MavenCoordinate::new("org.example", name, "1.0.0"),
            url: format!("http://127.0.0.1:1/{name}.jar"),
            cache_path,
        }
    }

    fn lockfile_with_plugin(name: &str, sha256: String) -> Lockfile {
        Lockfile {
            plugins: vec![PluginLock {
                name: name.to_owned(),
                maven: format!("org.example:{name}"),
                version: "1.0.0".to_owned(),
                sha256,
                url: format!("http://example.com/{name}.jar"),
            }],
            ..Default::default()
        }
    }

    fn lockfile_with_toolchain(
        version: &str,
        konanc_sha256: Option<&str>,
        jre_sha256: Option<&str>,
    ) -> Lockfile {
        Lockfile {
            toolchain: Some(ToolchainLock {
                konanc_version: version.to_owned(),
                konanc_tarball_sha256: konanc_sha256.map(str::to_owned),
                jre_tarball_sha256: jre_sha256.map(str::to_owned),
                detekt_version: None,
                detekt_jar_sha256: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn managed_artifact_reports_lockfile_drift_before_offline() {
        let net = konvoy_util::net::NetworkClient::new(true);
        let resolver = ArtifactResolver::new(&net, LockfileManager::new(true));

        let result =
            resolver.resolve_artifact(|| Ok(false), false, || EngineError::LintNotConfigured);

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn resolve_artifact_skips_pin_check_when_unlocked() {
        // The pin closure is evaluated only under --locked. When unlocked its
        // cost (e.g. stat-ing the install location) must be skipped entirely.
        let net = konvoy_util::net::NetworkClient::new(false);
        let resolver = ArtifactResolver::new(&net, LockfileManager::new(false));
        let called = Cell::new(false);

        let result = resolver.resolve_artifact(
            || {
                called.set(true);
                Ok(true)
            },
            true,
            || EngineError::LintNotConfigured,
        );

        assert!(result.is_ok());
        assert!(!called.get(), "pin check must be skipped when unlocked");
    }

    #[test]
    fn managed_artifact_maps_offline_unavailable() {
        let net = konvoy_util::net::NetworkClient::new(true);
        let resolver = ArtifactResolver::new(&net, LockfileManager::new(false));

        let result =
            resolver.resolve_artifact(|| Ok(true), false, || EngineError::LintNotConfigured);

        assert!(matches!(result, Err(EngineError::LintNotConfigured)));
    }

    #[test]
    fn managed_artifact_allows_locked_pinned_absent_when_online() {
        let net = konvoy_util::net::NetworkClient::new(false);
        let resolver = ArtifactResolver::new(&net, LockfileManager::new(true));

        let result =
            resolver.resolve_artifact(|| Ok(true), false, || EngineError::LintNotConfigured);

        assert!(result.is_ok());
    }

    #[test]
    fn changed_dependency_source_resolves_only_when_policy_allows_changes() {
        let online = konvoy_util::net::NetworkClient::new(false);
        let resolver = ArtifactResolver::new(&online, LockfileManager::new(false));

        let result = resolver.resolve_changed_dependency_source("dep", "expected", "actual");
        assert!(result.is_ok());

        let locked = ArtifactResolver::new(&online, LockfileManager::new(true))
            .resolve_changed_dependency_source("dep", "expected", "actual");
        assert!(matches!(
            locked,
            Err(EngineError::DependencyHashMismatch { name, expected, actual })
                if name == "dep" && expected == "expected" && actual == "actual"
        ));
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
    fn resolve_toolchain_reports_locked_drift_before_offline_absence() {
        let lockfile = Lockfile::default();

        let result = with_resolver(true, true, |resolver| {
            resolver.resolve_toolchain("0.0.0-resolver-toolchain-drift", &lockfile)
        });

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn resolve_toolchain_reports_offline_when_pinned_but_absent() {
        let version = "0.0.0-resolver-toolchain-offline";
        let lockfile = lockfile_with_toolchain(version, Some("konanc-sha"), Some("jre-sha"));

        let result = with_resolver(true, false, |resolver| {
            resolver.resolve_toolchain(version, &lockfile)
        });

        assert!(matches!(
            result,
            Err(EngineError::ToolchainOffline { version: got }) if got == version
        ));
    }

    #[test]
    fn resolve_detekt_jar_reports_locked_drift_when_unpinned() {
        let lockfile = Lockfile::default();

        let result = with_resolver(false, true, |resolver| {
            resolver.resolve_detekt_jar("99.99.99-resolver-unpinned", &lockfile)
        });

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn resolve_detekt_jar_reports_offline_when_pinned_but_absent() {
        let version = "99.99.99-resolver-offline";
        let lockfile = Lockfile {
            toolchain: Some(ToolchainLock {
                konanc_version: "2.1.0".to_owned(),
                konanc_tarball_sha256: None,
                jre_tarball_sha256: None,
                detekt_version: Some(version.to_owned()),
                detekt_jar_sha256: Some("0".repeat(64)),
            }),
            ..Default::default()
        };

        let result = with_resolver(true, false, |resolver| {
            resolver.resolve_detekt_jar(version, &lockfile)
        });

        assert!(matches!(
            result,
            Err(EngineError::DetektJarOffline { version: got }) if got == version
        ));
    }

    #[test]
    fn resolve_detekt_jre_reports_locked_drift_when_toolchain_hashes_missing() {
        let version = "0.0.0-resolver-detekt-jre-drift";
        let lockfile = lockfile_with_toolchain(version, None, None);

        let result = with_resolver(false, true, |resolver| {
            resolver.resolve_detekt_jre(version, &lockfile)
        });

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn resolve_detekt_jre_reports_offline_when_pinned_but_absent() {
        let version = "0.0.0-resolver-detekt-jre-offline";
        let lockfile = lockfile_with_toolchain(version, Some("konanc-sha"), Some("jre-sha"));

        let result = with_resolver(true, false, |resolver| {
            resolver.resolve_detekt_jre(version, &lockfile)
        });

        assert!(matches!(
            result,
            Err(EngineError::DetektJreOffline { version: got }) if got == version
        ));
    }

    #[test]
    fn prepare_plugin_artifacts_reports_missing_pin_before_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let artifacts = vec![plugin_artifact(
            "missing-pin",
            tmp.path().join("missing.jar"),
        )];
        let lockfile = Lockfile::default();

        let result = with_resolver(true, true, |resolver| {
            resolver.prepare_plugin_artifacts(&artifacts, &lockfile)
        });

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
    }

    #[test]
    fn prepare_plugin_artifacts_reports_offline_for_pinned_absent_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let artifacts = vec![plugin_artifact(
            "offline-plugin",
            tmp.path().join("plugin.jar"),
        )];
        let lockfile = lockfile_with_plugin("offline-plugin", "0".repeat(64));

        let result = with_resolver(true, false, |resolver| {
            resolver.prepare_plugin_artifacts(&artifacts, &lockfile)
        });

        assert!(matches!(
            result,
            Err(EngineError::PluginOffline { name }) if name == "offline-plugin"
        ));
    }

    #[test]
    fn prepare_plugin_artifacts_returns_cache_presence_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let present_path = tmp.path().join("present.jar");
        let absent_path = tmp.path().join("absent.jar");
        std::fs::write(&present_path, b"cached plugin").unwrap();
        let artifacts = vec![
            plugin_artifact("present-plugin", present_path),
            plugin_artifact("absent-plugin", absent_path),
        ];
        let mut lockfile = lockfile_with_plugin("present-plugin", "0".repeat(64));
        lockfile.plugins.push(PluginLock {
            name: "absent-plugin".to_owned(),
            maven: "org.example:absent-plugin".to_owned(),
            version: "1.0.0".to_owned(),
            sha256: "1".repeat(64),
            url: "http://example.com/absent-plugin.jar".to_owned(),
        });

        let present = with_resolver(false, true, |resolver| {
            resolver
                .prepare_plugin_artifacts(&artifacts, &lockfile)
                .unwrap()
        });

        assert_eq!(present, vec![true, false]);
    }

    #[test]
    fn resolve_plugin_artifact_returns_cached_hash_under_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_path = tmp.path().join("cached-plugin.jar");
        let content = b"cached plugin bytes";
        std::fs::write(&cache_path, content).unwrap();
        let expected_hash = konvoy_util::hash::sha256_bytes(content);
        let artifact = plugin_artifact("cached-plugin", cache_path.clone());
        let lockfile = lockfile_with_plugin("cached-plugin", expected_hash.clone());

        let result = with_resolver(true, false, |resolver| {
            resolver
                .resolve_plugin_artifact(&artifact, &lockfile, None)
                .unwrap()
        });

        assert_eq!(result.path, cache_path);
        assert_eq!(result.sha256, expected_hash);
        assert!(!result.freshly_downloaded);
    }

    #[test]
    fn resolve_maven_klib_reports_offline_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("missing.klib");

        let result = with_resolver(true, false, |resolver| {
            resolver.resolve_maven_klib(
                "missing-lib",
                "http://127.0.0.1:1/missing.klib",
                &dest,
                &"0".repeat(64),
                None,
            )
        });

        assert!(matches!(
            result,
            Err(EngineError::LibraryOffline { name }) if name == "missing-lib"
        ));
    }

    #[test]
    fn resolve_maven_klib_returns_cached_hash_under_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("cached.klib");
        let content = b"cached klib bytes";
        std::fs::write(&dest, content).unwrap();
        let expected_hash = konvoy_util::hash::sha256_bytes(content);

        let input = with_resolver(true, false, |resolver| {
            resolver
                .resolve_maven_klib(
                    "cached-lib",
                    "http://127.0.0.1:1/cached.klib",
                    &dest,
                    &expected_hash,
                    None,
                )
                .unwrap()
        });

        assert_eq!(input.path, dest);
        assert_eq!(
            input.precomputed_sha256.as_deref(),
            Some(expected_hash.as_str())
        );
    }

    #[test]
    fn cache_key_artifact_state_locked_does_not_compute_candidate() {
        let current = Lockfile::with_toolchain("2.1.0");
        let called = Cell::new(false);

        let effective = with_resolver(false, true, |resolver| {
            resolver.cache_key_artifact_state(&current, || {
                called.set(true);
                Lockfile::with_toolchain("9.9.9")
            })
        });

        assert_eq!(effective.toolchain.unwrap().konanc_version, "2.1.0");
        assert!(!called.get());
    }

    #[test]
    fn cache_key_artifact_state_unlocked_uses_candidate() {
        let current = Lockfile::with_toolchain("2.1.0");

        let effective = with_resolver(false, false, |resolver| {
            resolver.cache_key_artifact_state(&current, || Lockfile::with_toolchain("9.9.9"))
        });

        assert_eq!(effective.toolchain.unwrap().konanc_version, "9.9.9");
    }

    #[test]
    fn persist_resolved_artifacts_rejects_locked_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("konvoy.lock");
        let current = Lockfile::with_toolchain("2.1.0");
        let updated = Lockfile::with_toolchain("9.9.9");

        let result = with_resolver(false, true, |resolver| {
            resolver.persist_resolved_artifacts(&current, &updated, &path)
        });

        assert!(matches!(result, Err(EngineError::LockfileUpdateRequired)));
        assert!(!path.exists());
    }

    #[test]
    fn persist_resolved_artifacts_writes_unlocked_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("konvoy.lock");
        let current = Lockfile::with_toolchain("2.1.0");
        let updated = Lockfile::with_toolchain("9.9.9");

        with_resolver(false, false, |resolver| {
            resolver
                .persist_resolved_artifacts(&current, &updated, &path)
                .unwrap()
        });

        let reparsed = Lockfile::from_path(&path).unwrap();
        assert_eq!(reparsed.toolchain.unwrap().konanc_version, "9.9.9");
    }

    #[test]
    fn require_manifest_artifacts_resolvable_only_checks_when_locked() {
        let manifest = Manifest::from_str(
            "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"2.1.0\"\n",
            "konvoy.toml",
        )
        .unwrap();
        let lockfile = Lockfile::default();

        let unlocked = with_resolver(false, false, |resolver| {
            resolver.require_manifest_artifacts_resolvable(&manifest, &lockfile)
        });
        let locked = with_resolver(false, true, |resolver| {
            resolver.require_manifest_artifacts_resolvable(&manifest, &lockfile)
        });

        assert!(unlocked.is_ok());
        assert!(matches!(locked, Err(EngineError::LockfileUpdateRequired)));
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
