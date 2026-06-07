//! OpenAPI source generation using Fabrikt.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use konvoy_config::manifest::OpenApiCodegen;
use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

use crate::codegen::{run_java_jar, CodeGenerator, ManagedToolSpec};
use crate::error::EngineError;

const GENERATOR_NAME: &str = "openapi";
const TOOL_NAME: &str = "fabrikt";

/// OpenAPI generator backed by the Fabrikt CLI JAR.
#[derive(Debug, Clone)]
pub struct OpenApiGenerator {
    config: OpenApiCodegen,
}

impl OpenApiGenerator {
    /// Create a generator from parsed manifest config.
    #[must_use]
    pub fn new(config: OpenApiCodegen) -> Self {
        Self { config }
    }
}

fn fabrikt_tool(version: &str) -> ManagedToolSpec {
    ManagedToolSpec::maven_jar(
        TOOL_NAME,
        TOOL_NAME,
        MavenCoordinate::new("com.cjbooms", TOOL_NAME, version),
    )
}

/// Return the managed Fabrikt JAR path for `version`.
///
/// # Errors
/// Returns an error if the Konvoy home directory cannot be resolved.
pub fn fabrikt_jar_path(version: &str) -> Result<PathBuf, EngineError> {
    fabrikt_tool(version).artifact_path()
}

/// Return the Maven Central URL for a Fabrikt JAR.
#[must_use]
pub fn fabrikt_download_url(version: &str) -> String {
    MavenCoordinate::new("com.cjbooms", TOOL_NAME, version).to_url(MAVEN_CENTRAL)
}

/// Return whether Fabrikt is installed for `version`.
///
/// # Errors
/// Returns an error if the Konvoy home directory cannot be resolved.
pub fn is_installed(version: &str) -> Result<bool, EngineError> {
    fabrikt_tool(version).is_installed()
}

/// Download or verify the managed Fabrikt JAR.
///
/// # Errors
/// Returns an error if the version is unsafe, the artifact cannot be
/// downloaded, or the expected SHA-256 does not match.
pub fn ensure_fabrikt(
    version: &str,
    expected_sha256: Option<&str>,
) -> Result<(PathBuf, String), EngineError> {
    fabrikt_tool(version).ensure(expected_sha256)
}

impl CodeGenerator for OpenApiGenerator {
    fn name(&self) -> &str {
        GENERATOR_NAME
    }

    fn display_name(&self) -> &str {
        "OpenAPI"
    }

    fn managed_tool(&self) -> ManagedToolSpec {
        fabrikt_tool(&self.config.version)
    }

    fn config_hash_parts(&self) -> Vec<String> {
        vec![
            format!("tool_version={}", self.config.version),
            format!("spec={}", self.config.spec),
            format!("base_package={}", self.config.base_package),
        ]
    }

    fn input_files(&self) -> Vec<PathBuf> {
        vec![PathBuf::from(&self.config.spec)]
    }

    fn generate(
        &self,
        project_root: &Path,
        output_dir: &Path,
        tool_path: &Path,
        jre_home: &Path,
        verbose: bool,
    ) -> Result<(), EngineError> {
        let spec_path = project_root.join(&self.config.spec);
        eprintln!(
            "    Generating OpenAPI sources with Fabrikt {}...",
            self.config.version
        );

        run_java_jar(
            GENERATOR_NAME,
            "Fabrikt",
            tool_path,
            jre_home,
            vec![
                OsString::from("--api-file"),
                spec_path.into_os_string(),
                OsString::from("--base-package"),
                OsString::from(&self.config.base_package),
                OsString::from("--output-directory"),
                output_dir.as_os_str().to_owned(),
                OsString::from("--targets"),
                OsString::from("HTTP_MODELS"),
                OsString::from("--serialization-library"),
                OsString::from("KOTLINX_SERIALIZATION"),
            ],
            verbose,
        )
    }
}
