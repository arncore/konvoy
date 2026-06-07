//! OpenAPI source generation using Fabrikt.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use konvoy_config::manifest::OpenApiCodegen;
use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

use crate::codegen::{run_java_jar, CodeGenerator, ManagedToolSpec};
use crate::error::EngineError;

const GENERATOR_NAME: &str = "openapi";
const TOOL_NAME: &str = "fabrikt";

/// Fixed Fabrikt generation flags. These affect generated output, so they are
/// also folded into the config hash (see `config_hash_parts`) — if either ever
/// changes, the codegen hash and build cache key change with it.
const FABRIKT_TARGETS: &str = "HTTP_MODELS";
const FABRIKT_SERIALIZATION_LIBRARY: &str = "KOTLINX_SERIALIZATION";

/// Upper bound on the number of spec files (top-level + transitive `$ref`
/// targets) hashed for a single generator. A safety cap against pathological
/// or cyclic specs.
const MAX_SPEC_FILES: usize = 256;

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
            format!("targets={FABRIKT_TARGETS}"),
            format!("serialization_library={FABRIKT_SERIALIZATION_LIBRARY}"),
        ]
    }

    fn input_files(&self, project_root: &Path) -> Vec<PathBuf> {
        let spec_rel = PathBuf::from(&self.config.spec);
        let mut files = vec![spec_rel.clone()];
        collect_ref_files(project_root, &spec_rel, &mut files);
        files.sort();
        files.dedup();
        files
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
                OsString::from(FABRIKT_TARGETS),
                OsString::from("--serialization-library"),
                OsString::from(FABRIKT_SERIALIZATION_LIBRARY),
            ],
            verbose,
        )
    }
}

/// Best-effort transitive `$ref` file collector.
///
/// Scans an OpenAPI spec (YAML or JSON) for `$ref` targets that point to local
/// files and recursively follows them, appending every reachable spec file
/// (project-relative, lexically normalized) to `acc`. This lets a change to a
/// referenced sub-spec invalidate the codegen hash / build cache key.
///
/// Failures to read a file are skipped: hashing is best-effort, and the
/// top-level spec is always included by the caller. Internal refs (`#/...`) and
/// remote refs (`http(s)://`) are ignored.
fn collect_ref_files(project_root: &Path, spec_rel: &Path, acc: &mut Vec<PathBuf>) {
    let mut visited: std::collections::BTreeSet<PathBuf> =
        std::collections::BTreeSet::from([spec_rel.to_path_buf()]);
    let mut stack = vec![spec_rel.to_path_buf()];

    while let Some(current_rel) = stack.pop() {
        if acc.len() >= MAX_SPEC_FILES {
            break;
        }
        let Ok(text) = std::fs::read_to_string(project_root.join(&current_rel)) else {
            continue;
        };
        let base_dir = current_rel.parent().unwrap_or_else(|| Path::new(""));
        for raw_ref in extract_ref_targets(&text) {
            // Drop the JSON-pointer fragment; skip internal (`#/...`) refs.
            let file_part = raw_ref.split('#').next().unwrap_or("");
            if file_part.is_empty() || file_part.contains("://") {
                continue;
            }
            let resolved = normalize_relative(base_dir, file_part);
            if !visited.insert(resolved.clone()) {
                continue;
            }
            if !project_root.join(&resolved).is_file() {
                continue;
            }
            acc.push(resolved.clone());
            stack.push(resolved);
            if acc.len() >= MAX_SPEC_FILES {
                break;
            }
        }
    }
}

/// Extract raw `$ref` target strings from YAML/JSON spec text (best-effort,
/// format-agnostic). Handles quoted (`$ref: "x.yaml"`, `"$ref": "x.json"`) and
/// bare (`$ref: x.yaml`) forms.
fn extract_ref_targets(text: &str) -> Vec<String> {
    const MARKER: &str = "$ref";
    let mut refs = Vec::new();

    for (idx, _) in text.match_indices(MARKER) {
        // Everything after this `$ref` occurrence (`$ref` is ASCII, so the
        // offset lands on a char boundary).
        let Some(after) = text.get(idx + MARKER.len()..) else {
            continue;
        };
        // Skip the closing quote of `"$ref"` (JSON), whitespace, and the colon.
        let after = after.trim_start_matches(['"', '\'', ' ', '\t', ':']);
        let mut chars = after.chars();
        let target: String = match chars.next() {
            // Quoted value: take everything up to the matching quote.
            Some(quote @ ('"' | '\'')) => chars.take_while(|&c| c != quote).collect(),
            // Bare token: the first char plus everything up to a delimiter.
            Some(first) => std::iter::once(first)
                .chain(
                    chars.take_while(|&c| !matches!(c, ' ' | '\t' | '\r' | '\n' | ',' | '}' | ']')),
                )
                .collect(),
            None => continue,
        };
        let trimmed = target.trim();
        if !trimmed.is_empty() {
            refs.push(trimmed.to_owned());
        }
    }
    refs
}

/// Lexically resolve `rel` against `base_dir` (both project-relative),
/// collapsing `.` and `..` without touching the filesystem. Leading `..` that
/// would escape the project root are preserved as relative components (the path
/// is never made absolute).
fn normalize_relative(base_dir: &Path, rel: &str) -> PathBuf {
    use std::ffi::{OsStr, OsString};
    use std::path::Component;

    let mut stack: Vec<OsString> = Vec::new();
    for component in base_dir.components().chain(Path::new(rel).components()) {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(last) if last.as_os_str() != OsStr::new("..") => {
                    stack.pop();
                }
                _ => stack.push(OsString::from("..")),
            },
            Component::Normal(part) => stack.push(part.to_os_string()),
            // An absolute ref drops its root; only the normal parts are kept.
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    let mut out = PathBuf::new();
    for part in stack {
        out.push(part);
    }
    out
}
