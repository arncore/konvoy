//! OpenAPI source generation using Fabrikt.

use std::path::{Path, PathBuf};
use std::process::Command;

use konvoy_config::lockfile::{Lockfile, ToolchainLock};
use konvoy_config::manifest::OpenApiCodegen;
use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

use crate::codegen::{CodeGenerator, ToolResolution};
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

fn tools_dir() -> Result<PathBuf, EngineError> {
    Ok(konvoy_util::fs::konvoy_home()?.join("tools"))
}

fn fabrikt_dir(version: &str) -> Result<PathBuf, EngineError> {
    Ok(tools_dir()?.join(TOOL_NAME).join(version))
}

/// Return the managed Fabrikt JAR path for `version`.
///
/// # Errors
/// Returns an error if the Konvoy home directory cannot be resolved.
pub fn fabrikt_jar_path(version: &str) -> Result<PathBuf, EngineError> {
    Ok(fabrikt_dir(version)?.join(format!("fabrikt-{version}.jar")))
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
    Ok(fabrikt_jar_path(version)?.exists())
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
    konvoy_util::artifact::validate_version(version).map_err(|_| EngineError::CodegenDownload {
        name: TOOL_NAME.to_owned(),
        version: version.to_owned(),
        message: format!(
            "invalid fabrikt version \"{version}\" — only alphanumeric characters, dots, hyphens, and underscores are allowed"
        ),
    })?;

    let jar = fabrikt_jar_path(version)?;
    let url = fabrikt_download_url(version);
    let progress = (!jar.exists())
        .then(|| konvoy_util::progress::new_download_bar(format!("{TOOL_NAME} {version}")));
    let result =
        konvoy_util::progress::fetch(&url, &jar, expected_sha256, TOOL_NAME, progress.as_ref())
            .map_err(|e| map_download_err(version, e))?;
    if progress.is_some() {
        eprintln!();
    }

    Ok((result.path, result.sha256))
}

fn map_download_err(version: &str, e: konvoy_util::error::UtilError) -> EngineError {
    match e {
        konvoy_util::error::UtilError::Download { message } => EngineError::CodegenDownload {
            name: TOOL_NAME.to_owned(),
            version: version.to_owned(),
            message,
        },
        konvoy_util::error::UtilError::ArtifactHashMismatch {
            expected, actual, ..
        } => EngineError::CodegenHashMismatch {
            name: TOOL_NAME.to_owned(),
            version: version.to_owned(),
            expected,
            actual,
        },
        other => EngineError::Util(other),
    }
}

fn resolve_lockfile_hash<'a>(lockfile: &'a Lockfile, version: &str) -> Option<&'a str> {
    let toolchain = lockfile.toolchain.as_ref()?;
    let pinned_version = toolchain.fabrikt_version.as_deref()?;
    if pinned_version == version {
        toolchain.fabrikt_jar_sha256.as_deref()
    } else {
        None
    }
}

fn persist_fabrikt_hash(
    lockfile_path: &Path,
    lockfile: &Lockfile,
    kotlin_version: &str,
    version: &str,
    hash: &str,
) -> Result<(), EngineError> {
    let mut updated = lockfile.clone();
    if let Some(ref mut tc) = updated.toolchain {
        tc.fabrikt_version = Some(version.to_owned());
        tc.fabrikt_jar_sha256 = Some(hash.to_owned());
    } else {
        updated.toolchain = Some(ToolchainLock {
            konanc_version: kotlin_version.to_owned(),
            konanc_tarball_sha256: None,
            jre_tarball_sha256: None,
            detekt_version: None,
            detekt_jar_sha256: None,
            fabrikt_version: Some(version.to_owned()),
            fabrikt_jar_sha256: Some(hash.to_owned()),
        });
    }
    updated.write_to(lockfile_path)?;
    Ok(())
}

fn java_bin(jre_home: &Path) -> PathBuf {
    let binary = if cfg!(windows) { "java.exe" } else { "java" };
    jre_home.join("bin").join(binary)
}

impl CodeGenerator for OpenApiGenerator {
    fn name(&self) -> &str {
        GENERATOR_NAME
    }

    fn compute_input_hash(&self, project_root: &Path) -> Result<String, EngineError> {
        let spec_path = project_root.join(&self.config.spec);
        if !spec_path.exists() {
            return Err(EngineError::CodegenInputNotFound {
                name: GENERATOR_NAME.to_owned(),
                path: spec_path.display().to_string(),
            });
        }
        Ok(konvoy_util::hash::sha256_file(&spec_path)?)
    }

    fn ensure_tool(
        &self,
        lockfile: &Lockfile,
        locked: bool,
    ) -> Result<ToolResolution, EngineError> {
        let expected_hash = resolve_lockfile_hash(lockfile, &self.config.version);
        if locked {
            if expected_hash.is_none() {
                return Err(EngineError::LockfileUpdateRequired);
            }
            if !is_installed(&self.config.version)? {
                return Err(EngineError::CodegenToolNotFound {
                    name: TOOL_NAME.to_owned(),
                    version: self.config.version.clone(),
                });
            }
        }

        let (path, sha256) = ensure_fabrikt(&self.config.version, expected_hash)?;
        Ok(ToolResolution {
            path,
            sha256,
            should_persist: expected_hash.is_none(),
        })
    }

    fn persist_tool_hash(
        &self,
        lockfile_path: &Path,
        lockfile: &Lockfile,
        kotlin_version: &str,
        hash: &str,
    ) -> Result<(), EngineError> {
        persist_fabrikt_hash(
            lockfile_path,
            lockfile,
            kotlin_version,
            &self.config.version,
            hash,
        )
    }

    fn needs_tool_lock_update(&self, lockfile: &Lockfile) -> bool {
        resolve_lockfile_hash(lockfile, &self.config.version).is_none()
    }

    fn generate(
        &self,
        project_root: &Path,
        output_dir: &Path,
        tool_path: &Path,
        jre_home: &Path,
        verbose: bool,
    ) -> Result<(), EngineError> {
        let java = java_bin(jre_home);
        if !java.exists() {
            return Err(EngineError::CodegenFailed {
                name: GENERATOR_NAME.to_owned(),
                message: format!(
                    "java not found at {} — run `konvoy toolchain install` to reinstall the managed JRE",
                    java.display()
                ),
            });
        }

        let spec_path = project_root.join(&self.config.spec);
        eprintln!(
            "    Generating OpenAPI sources with Fabrikt {}...",
            self.config.version
        );

        let output = Command::new(&java)
            .arg("-jar")
            .arg(tool_path)
            .arg("--api-file")
            .arg(&spec_path)
            .arg("--base-package")
            .arg(&self.config.base_package)
            .arg("--output-directory")
            .arg(output_dir)
            .arg("--targets")
            .arg("HTTP_MODELS")
            .arg("--serialization-library")
            .arg("KOTLINX_SERIALIZATION")
            .env("JAVA_HOME", jre_home)
            .output()
            .map_err(|e| EngineError::CodegenFailed {
                name: GENERATOR_NAME.to_owned(),
                message: e.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if verbose {
            if !stdout.is_empty() {
                eprintln!("{stdout}");
            }
            if !stderr.is_empty() {
                eprintln!("{stderr}");
            }
        }

        if !output.status.success() {
            let raw = format!("{stdout}{stderr}");
            let hint = first_non_empty_line(&raw)
                .map(|line| format!(" first message: {line}"))
                .unwrap_or_default();
            return Err(EngineError::CodegenFailed {
                name: GENERATOR_NAME.to_owned(),
                message: format!(
                    "Fabrikt exited with status {}.{hint} Run with --verbose to see full output.",
                    output.status
                ),
            });
        }

        Ok(())
    }
}

fn first_non_empty_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}
