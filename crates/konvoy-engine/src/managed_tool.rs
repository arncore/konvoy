//! Managed JAR tools: download, cache, and SHA-verify a versioned JAR into
//! `~/.konvoy/tools/<id>/<version>/`. Shared by the detekt linter and the
//! codegen tools (e.g. Fabrikt) — both fetch a versioned JAR and run it via the
//! managed JRE.
//!
//! This type owns only *fetching and locating* the artifact. Domain-specific
//! error mapping (`DetektDownload` vs `CodegenDownload`, …) and lockfile pinning
//! stay with the caller: [`ensure`](ManagedToolSpec::ensure) returns the raw
//! [`UtilError`] so each caller can map it via `error::map_artifact_download_err`
//! and persist the hash wherever its lockfile section lives.

use std::path::PathBuf;

use konvoy_util::error::UtilError;
use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

/// Where a managed tool's JAR is fetched from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    /// A Maven Central artifact addressed by coordinate (e.g. Fabrikt).
    Maven(MavenCoordinate),
    /// A direct release URL with a fixed filename (e.g. detekt's GitHub release).
    DirectUrl { url: String, filename: String },
}

/// A managed JAR tool downloaded into `~/.konvoy/tools/<id>/<version>/`.
///
/// Fields are private so the validating constructors ([`maven_jar`](Self::maven_jar)
/// / [`direct_url`](Self::direct_url)) are the only construction path — callers
/// can't assemble a spec whose `version`/`filename` would escape the tools dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedToolSpec {
    /// Stable lockfile/cache id, e.g. `"fabrikt"` or `"detekt"`.
    id: String,
    /// Human-readable name used in progress output / diagnostics.
    display_name: String,
    /// Tool version.
    version: String,
    /// Where the JAR comes from.
    source: ToolSource,
}

impl ManagedToolSpec {
    /// A managed Maven Central JAR; the version is taken from the coordinate.
    #[must_use]
    pub fn maven_jar(id: &str, display_name: &str, coordinate: MavenCoordinate) -> Self {
        Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            version: coordinate.version.clone(),
            source: ToolSource::Maven(coordinate),
        }
    }

    /// A managed JAR fetched from a direct release URL with a fixed filename.
    #[must_use]
    pub fn direct_url(
        id: &str,
        display_name: &str,
        version: &str,
        url: String,
        filename: String,
    ) -> Self {
        Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            version: version.to_owned(),
            source: ToolSource::DirectUrl { url, filename },
        }
    }

    /// The stable lockfile/cache id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The tool version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The artifact filename on disk.
    #[must_use]
    pub fn filename(&self) -> String {
        match &self.source {
            ToolSource::Maven(coordinate) => coordinate.filename(),
            ToolSource::DirectUrl { filename, .. } => filename.clone(),
        }
    }

    /// The download URL for this tool.
    #[must_use]
    pub fn download_url(&self) -> String {
        match &self.source {
            ToolSource::Maven(coordinate) => coordinate.to_url(MAVEN_CENTRAL),
            ToolSource::DirectUrl { url, .. } => url.clone(),
        }
    }

    /// The managed artifact path under `~/.konvoy/tools/<id>/<version>/`.
    ///
    /// This is a pure path computation (read-only callers like `is_installed`
    /// rely on it not erroring on a malformed version). The traversal guard lives
    /// in [`ensure`](Self::ensure), the only method that *writes* — see `validate`.
    ///
    /// # Errors
    /// Returns an error if the Konvoy home directory cannot be resolved.
    pub fn artifact_path(&self) -> Result<PathBuf, UtilError> {
        Ok(konvoy_util::fs::konvoy_home()?
            .join("tools")
            .join(&self.id)
            .join(&self.version)
            .join(self.filename()))
    }

    /// Whether the artifact already exists locally.
    ///
    /// # Errors
    /// Returns an error if the Konvoy home directory cannot be resolved.
    pub fn is_installed(&self) -> Result<bool, UtilError> {
        Ok(self.artifact_path()?.exists())
    }

    /// Download (or verify a cached) artifact, returning `(path, sha256)`.
    ///
    /// Domain error mapping is the caller's job: map the returned `UtilError`
    /// via `error::map_artifact_download_err`.
    ///
    /// # Errors
    /// Returns an error if the id/version/filename is unsafe, the artifact cannot
    /// be downloaded, or the expected SHA-256 does not match.
    pub fn ensure(&self, expected_sha256: Option<&str>) -> Result<(PathBuf, String), UtilError> {
        // Validate here (the only method that writes), so a traversal-laden
        // version/filename can never escape the tools dir during a download.
        self.validate()?;

        let artifact_path = self.artifact_path()?;
        // Only show a download bar when the JAR isn't already cached — a cached
        // re-verify completes in milliseconds and the flash is more noise than
        // information.
        let progress = (!artifact_path.exists()).then(|| {
            konvoy_util::progress::new_download_bar(format!(
                "{} {}",
                self.display_name, self.version
            ))
        });
        // Pass the validated `id` (not the free-text display_name) as the fetch
        // label — download_artifact embeds it in a temp filename, so a separator
        // in display_name would point the temp path at a missing directory.
        let result = konvoy_util::progress::fetch(
            &self.download_url(),
            &artifact_path,
            expected_sha256,
            &self.id,
            progress.as_ref(),
        )?;
        if progress.is_some() {
            eprintln!();
        }

        Ok((result.path, result.sha256))
    }

    /// Reject any component that could escape `~/.konvoy/tools/<id>/<version>/`.
    /// `validate_identifier` allows `[A-Za-z0-9._-]` but rejects `..`, covering
    /// the id, version, and the on-disk filename (Maven `artifact-version.ext`
    /// or the caller-supplied direct-URL filename).
    fn validate(&self) -> Result<(), UtilError> {
        konvoy_util::artifact::validate_identifier(&self.id)?;
        konvoy_util::artifact::validate_identifier(&self.version)?;
        konvoy_util::artifact::validate_identifier(&self.filename())?;
        if let ToolSource::Maven(coordinate) = &self.source {
            konvoy_util::artifact::validate_identifier(&coordinate.group_id)?;
            konvoy_util::artifact::validate_identifier(&coordinate.artifact_id)?;
        }
        Ok(())
    }
}
