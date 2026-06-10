//! OpenAPI source generation using Fabrikt.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use konvoy_config::manifest::OpenApiCodegen;
use konvoy_util::maven::MavenCoordinate;
use konvoy_util::path::{has_hidden_component_under, relative_to};
use konvoy_util::text::first_non_blank_line;

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
    // also validates, but its raw error would map to a generic one. Use the same
    // `validate_identifier` the spec uses (it rejects `..`), so a traversal-laden
    // version yields this clear message rather than falling through to the generic
    // download mapping.
    konvoy_util::artifact::validate_identifier(version).map_err(|_| EngineError::CodegenDownload {
        name: TOOL_NAME.to_owned(),
        version: version.to_owned(),
        message: format!(
            "invalid fabrikt version \"{version}\" — only alphanumeric characters, dots, hyphens, and underscores are allowed, and it cannot be `..`"
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
        // Only non-file config that affects generated output. `spec` and
        // `extra_spec_dirs` are deliberately NOT folded in here: `input_files`
        // already contributes their (normalized, sorted, deduped) paths *and*
        // contents to the same generator hash. Repeating the raw config strings
        // would only over-invalidate the cache on cosmetic edits (a leading `./`,
        // an interior `.`) or a reorder of `extra_spec_dirs` — neither of which
        // changes the resolved input set or the generated sources.
        vec![
            format!("tool_version={}", self.config.version),
            format!("base_package={}", self.config.base_package),
            format!("targets={FABRIKT_TARGETS}"),
            format!("serialization_library={FABRIKT_SERIALIZATION_LIBRARY}"),
            format!("validation_library={FABRIKT_VALIDATION_LIBRARY}"),
        ]
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
        // Just collect the inputs — the framework sorts + de-duplicates before
        // hashing, so this need not. Paths are normalized to project-relative form
        // (via `relative_to`) so the same file referenced two ways (e.g. the spec
        // written `./specs/api.yaml` and again as a `specs` dir entry) collapses to
        // one when the framework de-duplicates, and so the cache key is portable.
        let mut files = vec![relative_to(project_root, Path::new(&self.config.spec))];

        for dir in &self.config.extra_spec_dirs {
            let dir_abs = project_root.join(dir);
            if !dir_abs.is_dir() {
                return Err(EngineError::CodegenInputDirNotFound {
                    name: GENERATOR_NAME.to_owned(),
                    path: dir_abs.display().to_string(),
                });
            }
            for file in konvoy_util::fs::collect_all_files(&dir_abs)? {
                // Skip hidden entries (a path component below the dir that starts
                // with `.`): OS/editor/VCS noise like `.DS_Store`, `.git/`, and
                // editor swap files, plus the `.konvoy` output dir when a spec dir
                // contains it (e.g. extra_spec_dirs = ["."]). These are never spec
                // input, and folding them in would make the cache key depend on
                // platform/editor state, breaking deterministic, stable hashing.
                if has_hidden_component_under(&dir_abs, &file) {
                    continue;
                }
                files.push(relative_to(project_root, &file));
            }
        }

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

        // Clear the output dir first: Fabrikt only writes/overwrites files for the
        // schemas in the current spec — it never prunes — so a model removed or
        // renamed in the spec would otherwise leave a stale `.kt` behind that still
        // gets compiled into the build. Recreating it makes each run produce exactly
        // the current source set.
        konvoy_util::fs::remove_dir_all_if_exists(output_dir).map_err(EngineError::from)?;
        konvoy_util::fs::ensure_dir(output_dir).map_err(EngineError::from)?;

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
            let hint = first_non_blank_line(hint_source)
                .map(|line| format!(" first message: {line}"))
                .unwrap_or_default();
            // Only suggest --verbose when it wasn't already on (if it was, capture()
            // already echoed the full output, so the suggestion would be misleading).
            let verbose_hint = if verbose {
                ""
            } else {
                " Run with --verbose to see full output."
            };
            return Err(EngineError::CodegenFailed {
                name: GENERATOR_NAME.to_owned(),
                message: format!("{TOOL_DISPLAY} failed.{hint}{verbose_hint}"),
            });
        }

        Ok(())
    }
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

    /// The canonical set of inputs this generator contributes: `input_files` then
    /// sort + de-duplicate, exactly as the framework does before hashing.
    /// `input_files` itself returns inputs unsorted and possibly with duplicates by
    /// design — determinism is the framework's job, not the generator's — so these
    /// tests assert *content*, not the implementation's return order.
    fn canonical_inputs(spec: &str, extra_spec_dirs: &[&str], root: &Path) -> Vec<PathBuf> {
        let mut files = generator(spec, extra_spec_dirs).input_files(root).unwrap();
        files.sort();
        files.dedup();
        files
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
    fn input_files_tracks_every_file_under_spec_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let specs = tmp.path().join("specs");
        std::fs::create_dir_all(specs.join("nested")).unwrap();
        std::fs::write(specs.join("api.yaml"), "openapi").unwrap();
        std::fs::write(specs.join("pet.yaml"), "Pet").unwrap();
        std::fs::write(specs.join("nested").join("owner.yaml"), "Owner").unwrap();
        // A non-spec file in the directory is also tracked (over-approximation).
        std::fs::write(specs.join("README.md"), "notes").unwrap();

        // Project-relative, every file tracked, primary spec not double-counted.
        assert_eq!(
            canonical_inputs("specs/api.yaml", &["specs"], tmp.path()),
            vec![
                PathBuf::from("specs/README.md"),
                PathBuf::from("specs/api.yaml"),
                PathBuf::from("specs/nested/owner.yaml"),
                PathBuf::from("specs/pet.yaml"),
            ]
        );
    }

    #[test]
    fn dot_slash_spec_collapses_against_listed_dir() {
        // A spec written with a leading `./` normalizes to the same relative path as
        // the dir-collected entry, so the framework's de-dup folds it to one input.
        let tmp = tempfile::tempdir().unwrap();
        let specs = tmp.path().join("specs");
        std::fs::create_dir_all(&specs).unwrap();
        std::fs::write(specs.join("api.yaml"), "openapi").unwrap();

        assert_eq!(
            canonical_inputs("./specs/api.yaml", &["specs"], tmp.path()),
            vec![PathBuf::from("specs/api.yaml")]
        );
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
    fn config_hash_parts_is_stable_across_spec_and_dir_cosmetics() {
        // spec / extra_spec_dirs are tracked by input_files (normalized + sorted +
        // content), NOT config_hash_parts — so a leading `./` on the spec or a
        // reorder of extra_spec_dirs must not change config_hash_parts (those edits
        // don't change the resolved input set or the generated sources).
        let a = generator("./specs/api.yaml", &["a", "b"]).config_hash_parts();
        let b = generator("specs/api.yaml", &["b", "a"]).config_hash_parts();
        assert_eq!(a, b);
        assert!(
            a.iter()
                .all(|p| !p.starts_with("spec=") && !p.starts_with("extra_spec_dir")),
            "config_hash_parts must not fold in raw spec/extra_spec_dir paths: {a:?}"
        );
    }

    #[test]
    fn config_hash_parts_pins_output_affecting_config() {
        // Pins every output-affecting field as load-bearing: the tool version,
        // base_package, and the three fixed Fabrikt flags. Removing or changing any
        // (e.g. re-enabling JAVAX_VALIDATION, which breaks native compilation) would
        // otherwise leave the cache key unchanged and silently reuse stale sources.
        assert_eq!(
            generator("specs/api.yaml", &[]).config_hash_parts(),
            vec![
                "tool_version=20.0.0".to_owned(),
                "base_package=com.example.api".to_owned(),
                "targets=HTTP_MODELS".to_owned(),
                "serialization_library=KOTLINX_SERIALIZATION".to_owned(),
                "validation_library=NO_VALIDATION".to_owned(),
            ]
        );
    }

    #[test]
    fn generate_clears_output_dir_before_running() {
        // generate() must clear stale output before invoking Fabrikt. Verify it even
        // when the run itself can't proceed: with no JRE the Fabrikt run errors, but
        // the pre-existing stale file must already be gone and the dir recreated.
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("Stale.kt"), "// from a previous run").unwrap();

        let result = generator("api.yaml", &[]).generate(tmp.path(), &out, None, false);
        assert!(result.is_err(), "a JVM run with no JRE should error");
        assert!(
            !out.join("Stale.kt").exists(),
            "stale output must be removed before the generator runs"
        );
        assert!(out.is_dir(), "output dir should be recreated");
    }

    #[test]
    fn extra_spec_dirs_order_does_not_change_the_input_set() {
        // Reordering extra_spec_dirs yields the same canonical input set, so the
        // codegen hash is unchanged. (input_files preserves dir order; the framework
        // sorts — this asserts the content is identical regardless.)
        let tmp = tempfile::tempdir().unwrap();
        for d in ["a", "b"] {
            std::fs::create_dir_all(tmp.path().join(d)).unwrap();
            std::fs::write(tmp.path().join(d).join("s.yaml"), d).unwrap();
        }
        std::fs::write(tmp.path().join("api.yaml"), "openapi").unwrap();

        assert_eq!(
            canonical_inputs("api.yaml", &["a", "b"], tmp.path()),
            canonical_inputs("api.yaml", &["b", "a"], tmp.path()),
        );
    }

    #[test]
    fn input_files_skips_hidden_files_and_dirs() {
        // OS/editor/VCS noise under a spec dir must not enter the cache key, or the
        // hash would flip on a `.DS_Store` write and differ across platforms.
        let tmp = tempfile::tempdir().unwrap();
        let specs = tmp.path().join("specs");
        std::fs::create_dir_all(specs.join(".git")).unwrap();
        std::fs::write(specs.join("api.yaml"), "openapi").unwrap();
        std::fs::write(specs.join(".DS_Store"), "junk").unwrap();
        std::fs::write(specs.join(".git").join("config"), "vcs").unwrap();

        // Only the real spec survives — the hidden entries are excluded (otherwise
        // they'd appear here alongside specs/api.yaml).
        assert_eq!(
            canonical_inputs("specs/api.yaml", &["specs"], tmp.path()),
            vec![PathBuf::from("specs/api.yaml")]
        );
    }

    #[test]
    fn input_files_with_dot_dir_excludes_konvoy_output_dir() {
        // extra_spec_dirs = ["."] walks the whole project; the generator's own
        // output under `.konvoy` (a hidden dir) must be excluded, or the input hash
        // would depend on its own output (self-referential cache key).
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("api.yaml"), "openapi").unwrap();
        let gen_out = tmp.path().join(".konvoy").join("gen").join("openapi");
        std::fs::create_dir_all(&gen_out).unwrap();
        std::fs::write(gen_out.join("Models.kt"), "generated").unwrap();

        assert_eq!(
            canonical_inputs("api.yaml", &["."], tmp.path()),
            vec![PathBuf::from("api.yaml")]
        );
    }

    #[test]
    fn input_files_empty_extra_spec_dir_contributes_nothing() {
        // A configured-but-empty spec dir is valid (no error) and adds no inputs,
        // so it can't perturb the hash until a file actually appears in it.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("api.yaml"), "openapi").unwrap();
        std::fs::create_dir_all(tmp.path().join("extra")).unwrap();

        let files = generator("api.yaml", &["extra"])
            .input_files(tmp.path())
            .unwrap();
        assert_eq!(files, vec![PathBuf::from("api.yaml")]);
    }
}
