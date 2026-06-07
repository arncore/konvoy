//! Managed codegen tool artifacts.

use std::path::PathBuf;

use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

use crate::error::EngineError;

/// A managed codegen tool artifact downloaded from Maven Central.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedToolSpec {
    /// Stable lockfile/cache id, e.g. `"fabrikt"` or `"grpc"`.
    pub id: String,
    /// Human-readable name used in diagnostics.
    pub display_name: String,
    /// Maven coordinate for the artifact.
    pub coordinate: MavenCoordinate,
}

/// Result of resolving a managed codegen tool.
#[derive(Debug, Clone)]
pub struct ManagedToolResolution {
    /// Path to the executable artifact, usually a JAR.
    pub path: PathBuf,
    /// SHA-256 hash of the tool artifact.
    pub sha256: String,
    /// Whether the hash should be persisted into `konvoy.lock`.
    pub should_persist: bool,
}

impl ManagedToolSpec {
    /// Create a managed Maven JAR tool spec.
    #[must_use]
    pub fn maven_jar(id: &str, display_name: &str, coordinate: MavenCoordinate) -> Self {
        Self {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            coordinate,
        }
    }

    /// Return the configured tool version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.coordinate.version
    }

    /// Return the Maven Central download URL for this tool.
    #[must_use]
    pub fn download_url(&self) -> String {
        self.coordinate.to_url(MAVEN_CENTRAL)
    }

    /// Return the managed artifact path under `~/.konvoy/tools/`.
    ///
    /// # Errors
    /// Returns an error if the Konvoy home directory cannot be resolved.
    pub fn artifact_path(&self) -> Result<PathBuf, EngineError> {
        Ok(konvoy_util::fs::konvoy_home()?
            .join("tools")
            .join(&self.id)
            .join(self.version())
            .join(self.coordinate.filename()))
    }

    /// Return whether the tool artifact exists locally.
    ///
    /// # Errors
    /// Returns an error if the Konvoy home directory cannot be resolved.
    pub fn is_installed(&self) -> Result<bool, EngineError> {
        Ok(self.artifact_path()?.exists())
    }

    /// Download or verify the managed tool artifact.
    ///
    /// # Errors
    /// Returns an error if the tool id/version is unsafe, the artifact cannot
    /// be downloaded, or the expected SHA-256 does not match.
    pub fn ensure(&self, expected_sha256: Option<&str>) -> Result<(PathBuf, String), EngineError> {
        self.validate()?;

        let artifact_path = self.artifact_path()?;
        let progress = (!artifact_path.exists()).then(|| {
            konvoy_util::progress::new_download_bar(format!(
                "{} {}",
                self.display_name,
                self.version()
            ))
        });
        let result = konvoy_util::progress::fetch(
            &self.download_url(),
            &artifact_path,
            expected_sha256,
            &self.display_name,
            progress.as_ref(),
        )
        .map_err(|e| self.map_download_err(e))?;
        if progress.is_some() {
            eprintln!();
        }

        Ok((result.path, result.sha256))
    }

    fn validate(&self) -> Result<(), EngineError> {
        validate_tool_part(self, "tool id", &self.id)?;
        let version_label = format!("{} version", self.id);
        validate_tool_part(self, &version_label, self.version())?;
        validate_tool_part(self, "group id", &self.coordinate.group_id)?;
        validate_tool_part(self, "artifact id", &self.coordinate.artifact_id)?;
        Ok(())
    }

    fn map_download_err(&self, e: konvoy_util::error::UtilError) -> EngineError {
        match e {
            konvoy_util::error::UtilError::Download { message } => EngineError::CodegenDownload {
                name: self.id.clone(),
                version: self.version().to_owned(),
                message,
            },
            konvoy_util::error::UtilError::ArtifactHashMismatch {
                expected, actual, ..
            } => EngineError::CodegenHashMismatch {
                name: self.id.clone(),
                version: self.version().to_owned(),
                expected,
                actual,
            },
            other => EngineError::Util(other),
        }
    }
}

fn validate_tool_part(spec: &ManagedToolSpec, label: &str, value: &str) -> Result<(), EngineError> {
    konvoy_util::artifact::validate_identifier(value).map_err(|_| EngineError::CodegenDownload {
        name: spec.id.clone(),
        version: spec.version().to_owned(),
        message: format!(
            "invalid {label} \"{value}\" — only alphanumeric characters, dots, hyphens, and underscores are allowed"
        ),
    })
}
