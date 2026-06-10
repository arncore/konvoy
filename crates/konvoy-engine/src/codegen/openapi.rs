//! OpenAPI source generation using Fabrikt.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use konvoy_config::manifest::OpenApiCodegen;
use konvoy_util::maven::MavenCoordinate;

use crate::codegen::{CodeGenerator, ManagedToolSpec};
use crate::error::EngineError;

const GENERATOR_NAME: &str = "openapi";
const TOOL_NAME: &str = "fabrikt";
/// Human-readable tool name, used consistently in the download bar, the
/// generation message, and failure diagnostics.
const TOOL_DISPLAY: &str = "Fabrikt";

/// Fixed Fabrikt generation flags. These affect generated output, so they are
/// also folded into the config hash (see `config_hash_parts`) — if any of them
/// ever changes, the codegen hash and build cache key change with it.
const FABRIKT_TARGETS: &str = "HTTP_MODELS";
const FABRIKT_SERIALIZATION_LIBRARY: &str = "KOTLINX_SERIALIZATION";
/// Fabrikt defaults to `JAVAX_VALIDATION`, which emits `javax.validation.*`
/// annotations on required/constrained fields. Those are JVM-only and do not
/// exist on Kotlin/Native, so generated models would not compile. Konvoy is
/// native-first, so validation annotations are disabled.
const FABRIKT_VALIDATION_LIBRARY: &str = "NO_VALIDATION";

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
        TOOL_DISPLAY,
        MavenCoordinate::new("com.cjbooms", TOOL_NAME, version),
    )
}

/// Return the managed Fabrikt JAR path for `version`.
///
/// # Errors
/// Returns an error if the Konvoy home directory cannot be resolved.
pub fn fabrikt_jar_path(version: &str) -> Result<PathBuf, EngineError> {
    fabrikt_tool(version)
        .artifact_path()
        .map_err(EngineError::from)
}

/// Return the Maven Central URL for a Fabrikt JAR.
#[must_use]
pub fn fabrikt_download_url(version: &str) -> String {
    fabrikt_tool(version).download_url()
}

/// Return whether Fabrikt is installed for `version`.
///
/// # Errors
/// Returns an error if the Konvoy home directory cannot be resolved.
pub fn is_installed(version: &str) -> Result<bool, EngineError> {
    fabrikt_tool(version)
        .is_installed()
        .map_err(EngineError::from)
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
    // Pre-validate for an actionable, Fabrikt-specific message; the shared tool
    // also validates, but its raw error would map to a generic one.
    konvoy_util::artifact::validate_version(version).map_err(|_| EngineError::CodegenDownload {
        name: TOOL_NAME.to_owned(),
        version: version.to_owned(),
        message: format!(
            "invalid fabrikt version \"{version}\" — only alphanumeric characters, dots, hyphens, and underscores are allowed"
        ),
    })?;
    fabrikt_tool(version)
        .ensure(expected_sha256)
        .map_err(|e| map_fabrikt_download_err(version, e))
}

/// Map a download/verify `UtilError` to the Fabrikt-specific engine error
/// (mirrors the detekt and plugin paths via the shared mapper).
fn map_fabrikt_download_err(version: &str, e: konvoy_util::error::UtilError) -> EngineError {
    crate::error::map_artifact_download_err(
        TOOL_NAME,
        e,
        |name, message| EngineError::CodegenDownload {
            name,
            version: version.to_owned(),
            message,
        },
        |name, expected, actual| EngineError::CodegenHashMismatch {
            name,
            version: version.to_owned(),
            expected,
            actual,
        },
    )
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
        let mut parts = vec![
            format!("tool_version={}", self.config.version),
            format!("spec={}", self.config.spec),
            format!("base_package={}", self.config.base_package),
            format!("targets={FABRIKT_TARGETS}"),
            format!("serialization_library={FABRIKT_SERIALIZATION_LIBRARY}"),
            format!("validation_library={FABRIKT_VALIDATION_LIBRARY}"),
        ];
        // One part per dir (plus a count) rather than a joined string: joining
        // with `,` is ambiguous — ["a,b"] and ["a","b"] would render identically,
        // and sha256_multi only length-prefixes whole parts.
        parts.push(format!(
            "extra_spec_dirs_count={}",
            self.config.extra_spec_dirs.len()
        ));
        for dir in &self.config.extra_spec_dirs {
            parts.push(format!("extra_spec_dir={dir}"));
        }
        parts
    }

    /// Project-relative files whose contents feed the codegen cache key.
    ///
    /// Always includes the primary `spec`. When `extra_spec_dirs` is configured, also
    /// includes every file under those directories so a change to any `$ref`'d
    /// sibling (which Fabrikt resolves internally but never reports) regenerates
    /// sources. Fabrikt exposes no resolved-input list — via CLI, library, or its
    /// Gradle plugins — so we deliberately over-approximate by directory rather
    /// than re-parse the spec in Rust.
    fn input_files(&self, project_root: &Path) -> Result<Vec<PathBuf>, EngineError> {
        // Normalize the spec the same way dir-collected files are normalized so a
        // spec written as `./specs/api.yaml` dedups against a listed `specs` dir
        // entry (`specs/api.yaml`) instead of hashing the same file twice.
        let mut files = vec![project_relative(project_root, Path::new(&self.config.spec))];

        for dir in &self.config.extra_spec_dirs {
            let dir_abs = project_root.join(dir);
            if !dir_abs.is_dir() {
                return Err(EngineError::CodegenInputDirNotFound {
                    name: GENERATOR_NAME.to_owned(),
                    path: dir_abs.display().to_string(),
                });
            }
            for file in konvoy_util::fs::collect_all_files(&dir_abs)? {
                files.push(project_relative(project_root, &file));
            }
        }

        files.sort();
        files.dedup();
        Ok(files)
    }

    fn generate(
        &self,
        project_root: &Path,
        output_dir: &Path,
        jre_home: Option<&Path>,
        verbose: bool,
    ) -> Result<(), EngineError> {
        let spec_path = project_root.join(&self.config.spec);
        eprintln!(
            "    Generating OpenAPI sources with {TOOL_DISPLAY} {}...",
            self.config.version
        );

        let args = [
            OsString::from("--api-file"),
            spec_path.into_os_string(),
            OsString::from("--base-package"),
            OsString::from(&self.config.base_package),
            OsString::from("--output-directory"),
            output_dir.as_os_str().to_owned(),
            OsString::from("--targets"),
            OsString::from(FABRIKT_TARGETS),
            OsString::from("--serialization-library"),
            OsString::from(FABRIKT_SERIALIZATION_LIBRARY),
            OsString::from("--validation-library"),
            OsString::from(FABRIKT_VALIDATION_LIBRARY),
        ];

        let output = self.managed_tool().run(jre_home, &args, verbose)?;

        // A code generator treats a non-zero exit as a hard failure (unlike the
        // linter, which reads it as "issues found").
        if !output.success {
            // JVM CLIs typically print the real error to stderr (and banners to
            // stdout), so prefer stderr for the one-line hint and fall back to
            // stdout. Never concatenate the streams — that can fuse an unrelated
            // stdout line onto the first stderr line.
            let hint_source = if output.stderr.trim().is_empty() {
                &output.stdout
            } else {
                &output.stderr
            };
            let hint = first_non_empty_line(hint_source)
                .map(|line| format!(" first message: {line}"))
                .unwrap_or_default();
            return Err(EngineError::CodegenFailed {
                name: GENERATOR_NAME.to_owned(),
                message: format!(
                    "{TOOL_DISPLAY} failed.{hint} Run with --verbose to see full output."
                ),
            });
        }

        Ok(())
    }
}

/// First non-blank line of `output`, trimmed — used to surface a concise hint
/// from a failed Fabrikt run without dumping its whole log.
fn first_non_empty_line(output: &str) -> Option<&str> {
    output.lines().map(str::trim).find(|line| !line.is_empty())
}

/// Normalize a path to a clean project-relative form for stable, dedup-able
/// hashing. Absolute inputs (dir-walk results under `project_root`) and relative
/// inputs (the configured `spec`, possibly written with a leading `./`) both
/// collapse to the same `specs/api.yaml`-style path. Joining onto `project_root`
/// then stripping it also drops interior `.` components via `Path::components`.
fn project_relative(project_root: &Path, path: &Path) -> PathBuf {
    let joined = project_root.join(path);
    joined
        .strip_prefix(project_root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn generator(spec: &str, extra_spec_dirs: &[&str]) -> OpenApiGenerator {
        OpenApiGenerator::new(OpenApiCodegen {
            version: "20.0.0".to_owned(),
            spec: spec.to_owned(),
            base_package: "com.example.api".to_owned(),
            extra_spec_dirs: extra_spec_dirs.iter().map(|s| (*s).to_owned()).collect(),
        })
    }

    #[test]
    fn input_files_without_extra_spec_dirs_is_just_the_spec() {
        let tmp = tempfile::tempdir().unwrap();
        let files = generator("specs/api.yaml", &[])
            .input_files(tmp.path())
            .unwrap();
        assert_eq!(files, vec![PathBuf::from("specs/api.yaml")]);
    }

    #[test]
    fn input_files_with_spec_dir_hashes_every_file_relative_and_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let specs = tmp.path().join("specs");
        std::fs::create_dir_all(specs.join("nested")).unwrap();
        std::fs::write(specs.join("api.yaml"), "openapi").unwrap();
        std::fs::write(specs.join("pet.yaml"), "Pet").unwrap();
        std::fs::write(specs.join("nested").join("owner.yaml"), "Owner").unwrap();
        // A non-spec file in the directory is also tracked (over-approximation).
        std::fs::write(specs.join("README.md"), "notes").unwrap();

        let files = generator("specs/api.yaml", &["specs"])
            .input_files(tmp.path())
            .unwrap();

        // Project-relative, sorted, and the primary spec is not duplicated.
        assert_eq!(
            files,
            vec![
                PathBuf::from("specs/README.md"),
                PathBuf::from("specs/api.yaml"),
                PathBuf::from("specs/nested/owner.yaml"),
                PathBuf::from("specs/pet.yaml"),
            ]
        );
    }

    #[test]
    fn input_files_dedups_dot_slash_spec_against_listed_dir() {
        // A spec written with a leading `./` must collapse to the same relative
        // path as the dir-collected entry, so the file is hashed once, not twice.
        let tmp = tempfile::tempdir().unwrap();
        let specs = tmp.path().join("specs");
        std::fs::create_dir_all(&specs).unwrap();
        std::fs::write(specs.join("api.yaml"), "openapi").unwrap();

        let files = generator("./specs/api.yaml", &["specs"])
            .input_files(tmp.path())
            .unwrap();
        assert_eq!(files, vec![PathBuf::from("specs/api.yaml")]);
    }

    #[test]
    fn input_files_missing_spec_dir_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let err = generator("specs/api.yaml", &["does-not-exist"])
            .input_files(tmp.path())
            .unwrap_err();
        assert!(
            matches!(err, EngineError::CodegenInputDirNotFound { .. }),
            "expected CodegenInputDirNotFound, got: {err:?}"
        );
    }

    #[test]
    fn extra_spec_dirs_participate_in_config_hash() {
        let with = generator("specs/api.yaml", &["specs"]).config_hash_parts();
        let without = generator("specs/api.yaml", &[]).config_hash_parts();
        assert_ne!(with, without);
        assert!(with.iter().any(|p| p == "extra_spec_dir=specs"));
        assert!(with.iter().any(|p| p == "extra_spec_dirs_count=1"));
        assert!(without.iter().any(|p| p == "extra_spec_dirs_count=0"));
    }

    #[test]
    fn config_hash_distinguishes_comma_dir_from_two_dirs() {
        // ["a,b"] must not collide with ["a","b"] in the config hash.
        let one = generator("specs/api.yaml", &["a,b"]).config_hash_parts();
        let two = generator("specs/api.yaml", &["a", "b"]).config_hash_parts();
        assert_ne!(one, two);
    }
}
