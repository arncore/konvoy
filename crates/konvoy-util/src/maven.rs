//! Maven coordinate parsing and URL generation.

use std::path::{Path, PathBuf};

use crate::error::UtilError;

/// Maven Central repository URL.
pub const MAVEN_CENTRAL: &str = "https://repo1.maven.org/maven2";

/// A parsed Maven coordinate identifying a single artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenCoordinate {
    /// Maven group identifier, e.g. `"org.jetbrains.kotlinx"`.
    pub group_id: String,
    /// Maven artifact identifier, e.g. `"kotlinx-serialization-core"`.
    pub artifact_id: String,
    /// Artifact version, e.g. `"1.8.0"`.
    pub version: String,
    /// File extension / packaging type (defaults to `"jar"`).
    pub packaging: String,
}

impl MavenCoordinate {
    /// Create a new coordinate with default packaging ("jar").
    pub fn new(group_id: &str, artifact_id: &str, version: &str) -> Self {
        Self {
            group_id: group_id.to_owned(),
            artifact_id: artifact_id.to_owned(),
            version: version.to_owned(),
            packaging: "jar".to_owned(),
        }
    }

    /// Builder method to override the packaging type.
    pub fn with_packaging(mut self, packaging: &str) -> Self {
        self.packaging = packaging.to_owned();
        self
    }

    /// Parse a Maven coordinate string.
    ///
    /// Accepted formats:
    /// - `"group:artifact:version"` (3 parts, packaging defaults to "jar")
    /// - `"group:artifact:version:packaging"` (4 parts)
    ///
    /// # Errors
    /// Returns `UtilError::InvalidMavenCoordinate` when the string has fewer
    /// than 3 or more than 4 colon-separated parts, or any part is empty.
    pub fn parse(coord: &str) -> Result<Self, UtilError> {
        let parts: Vec<&str> = coord.split(':').collect();

        if parts.len() < 3 {
            return Err(UtilError::InvalidMavenCoordinate {
                coordinate: coord.to_owned(),
                reason: format!(
                    "expected at least 3 colon-separated parts (group:artifact:version), got {}",
                    parts.len()
                ),
            });
        }

        if parts.len() > 4 {
            return Err(UtilError::InvalidMavenCoordinate {
                coordinate: coord.to_owned(),
                reason: format!(
                    "expected at most 4 colon-separated parts (group:artifact:version:packaging), got {}",
                    parts.len()
                ),
            });
        }

        // Check for empty parts.
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                let label = match i {
                    0 => "group_id",
                    1 => "artifact_id",
                    2 => "version",
                    3 => "packaging",
                    _ => "part",
                };
                return Err(UtilError::InvalidMavenCoordinate {
                    coordinate: coord.to_owned(),
                    reason: format!("{label} is empty"),
                });
            }
        }

        let (Some(group), Some(artifact), Some(version)) =
            (parts.first(), parts.get(1), parts.get(2))
        else {
            // Unreachable: we checked parts.len() >= 3 above.
            return Err(UtilError::InvalidMavenCoordinate {
                coordinate: coord.to_owned(),
                reason: "expected at least 3 parts".to_owned(),
            });
        };

        let mut result = Self::new(group, artifact, version);
        if let Some(pkg) = parts.get(3) {
            result.packaging = (*pkg).to_owned();
        }
        Ok(result)
    }

    /// The filename for this artifact: `"{artifact_id}-{version}.{packaging}"`.
    pub fn filename(&self) -> String {
        format!("{}-{}.{}", self.artifact_id, self.version, self.packaging)
    }

    /// The repository-relative path for this artifact.
    ///
    /// Dots in `group_id` are replaced with `/`, then:
    /// `"{group_path}/{artifact_id}/{version}/{filename}"`.
    pub fn repository_path(&self) -> String {
        let group_path = self.group_id.replace('.', "/");
        format!(
            "{}/{}/{}/{}",
            group_path,
            self.artifact_id,
            self.version,
            self.filename()
        )
    }

    /// Build the full download URL for this artifact.
    ///
    /// Strips any trailing `/` from `registry` before appending the path.
    pub fn to_url(&self, registry: &str) -> String {
        let base = registry.trim_end_matches('/');
        format!("{}/{}", base, self.repository_path())
    }

    /// Return the local cache path for this artifact, rooted at `cache_root`.
    ///
    /// Uses the same directory layout as `repository_path` but as a `PathBuf`.
    pub fn cache_path(&self, cache_root: &Path) -> PathBuf {
        let group_path = self.group_id.replace('.', "/");
        cache_root
            .join(group_path)
            .join(&self.artifact_id)
            .join(&self.version)
            .join(self.filename())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn parse_three_part() {
        let coord =
            MavenCoordinate::parse("org.jetbrains.kotlinx:kotlinx-serialization-core:1.8.0")
                .unwrap();
        assert_eq!(coord.group_id, "org.jetbrains.kotlinx");
        assert_eq!(coord.artifact_id, "kotlinx-serialization-core");
        assert_eq!(coord.version, "1.8.0");
        assert_eq!(coord.packaging, "jar");
    }

    #[test]
    fn parse_four_part() {
        let coord =
            MavenCoordinate::parse("org.jetbrains.kotlinx:kotlinx-serialization-core:1.8.0:klib")
                .unwrap();
        assert_eq!(coord.group_id, "org.jetbrains.kotlinx");
        assert_eq!(coord.artifact_id, "kotlinx-serialization-core");
        assert_eq!(coord.version, "1.8.0");
        assert_eq!(coord.packaging, "klib");
    }

    #[test]
    fn parse_rejects_two_parts() {
        let result = MavenCoordinate::parse("org.jetbrains:artifact");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid Maven coordinate"), "error was: {err}");
    }

    #[test]
    fn parse_rejects_five_parts() {
        let result = MavenCoordinate::parse("a:b:c:d:e");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid Maven coordinate"), "error was: {err}");
    }

    #[test]
    fn parse_rejects_empty_parts() {
        let result = MavenCoordinate::parse("org.jetbrains::1.0");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid Maven coordinate"), "error was: {err}");
        assert!(err.contains("empty"), "error was: {err}");
    }

    #[test]
    fn filename_jar() {
        let coord = MavenCoordinate::new("org.example", "artifact", "1.0.0");
        assert_eq!(coord.filename(), "artifact-1.0.0.jar");
    }

    #[test]
    fn filename_klib() {
        let coord = MavenCoordinate::new("org.example", "artifact", "1.0.0").with_packaging("klib");
        assert_eq!(coord.filename(), "artifact-1.0.0.klib");
    }

    #[test]
    fn repository_path_dots_to_slashes() {
        let coord = MavenCoordinate::new(
            "org.jetbrains.kotlinx",
            "kotlinx-serialization-core",
            "1.8.0",
        );
        let path = coord.repository_path();
        assert_eq!(
            path,
            "org/jetbrains/kotlinx/kotlinx-serialization-core/1.8.0/kotlinx-serialization-core-1.8.0.jar"
        );
    }

    #[test]
    fn to_url_maven_central() {
        let coord = MavenCoordinate::new(
            "org.jetbrains.kotlinx",
            "kotlinx-serialization-core",
            "1.8.0",
        );
        let url = coord.to_url(MAVEN_CENTRAL);
        assert_eq!(
            url,
            "https://repo1.maven.org/maven2/org/jetbrains/kotlinx/kotlinx-serialization-core/1.8.0/kotlinx-serialization-core-1.8.0.jar"
        );
    }

    #[test]
    fn to_url_custom_registry() {
        let coord = MavenCoordinate::new("com.example", "mylib", "2.0.0");

        // Without trailing slash.
        let url1 = coord.to_url("https://my.repo.com/maven");
        // With trailing slash.
        let url2 = coord.to_url("https://my.repo.com/maven/");

        assert_eq!(url1, url2);
        assert_eq!(
            url1,
            "https://my.repo.com/maven/com/example/mylib/2.0.0/mylib-2.0.0.jar"
        );
    }

    #[test]
    fn cache_path_layout() {
        let coord = MavenCoordinate::new(
            "org.jetbrains.kotlinx",
            "kotlinx-serialization-core",
            "1.8.0",
        );
        let cache = coord.cache_path(Path::new("/home/user/.konvoy/cache"));
        assert_eq!(
            cache,
            Path::new("/home/user/.konvoy/cache/org/jetbrains/kotlinx/kotlinx-serialization-core/1.8.0/kotlinx-serialization-core-1.8.0.jar")
        );
    }

    #[test]
    fn with_packaging_overrides() {
        let coord = MavenCoordinate::new("org.example", "mylib", "1.0.0").with_packaging("klib");
        assert_eq!(coord.packaging, "klib");
        assert_eq!(coord.filename(), "mylib-1.0.0.klib");
    }
}
