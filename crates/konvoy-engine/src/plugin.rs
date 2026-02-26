//! Data-driven plugin system for Kotlin compiler plugins.
//!
//! Each plugin is defined by a TOML descriptor file compiled into the binary.
//! The engine generically resolves, downloads, verifies, and wires plugin
//! artifacts into `konanc` invocation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Deserialize;

use konvoy_config::lockfile::{Lockfile, PluginLock};
use konvoy_config::manifest::Manifest;
use konvoy_targets::Target;
use konvoy_util::maven::{MavenCoordinate, MAVEN_CENTRAL};

use crate::error::EngineError;

/// A plugin descriptor loaded from a compiled-in TOML file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDescriptor {
    /// Plugin name (e.g. `"serialization"`).
    pub name: String,
    /// Maven coordinate template for the compiler plugin JAR.
    /// May contain `{kotlin_version}` placeholder.
    pub compiler_plugin: String,
    /// Named modules that ship runtime klibs for this plugin.
    #[serde(default)]
    pub modules: BTreeMap<String, ModuleSpec>,
}

/// Specification for a single runtime module within a plugin descriptor.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSpec {
    /// Maven coordinate template for this module.
    /// May contain `{version}` and `{target}` placeholders.
    pub maven: String,
    /// Whether this module is always included when the plugin is enabled.
    #[serde(default)]
    pub always: bool,
    /// Module names that this module depends on (transitive inclusion).
    #[serde(default)]
    pub depends_on: Vec<String>,
}

/// The kind of a resolved plugin artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginArtifactKind {
    /// A compiler plugin JAR passed via `-Xplugin=`.
    CompilerPlugin,
    /// A runtime klib passed via `-library`.
    Runtime,
}

/// A fully resolved plugin artifact ready for download.
#[derive(Debug, Clone)]
pub struct ResolvedPluginArtifact {
    /// The parent plugin name.
    pub plugin_name: String,
    /// Human-readable artifact label (e.g. `"compiler-plugin"` or module name).
    pub artifact_name: String,
    /// Whether this is a compiler plugin or runtime library.
    pub kind: PluginArtifactKind,
    /// Parsed Maven coordinate for this artifact.
    pub maven_coord: MavenCoordinate,
    /// Full download URL.
    pub url: String,
    /// Local cache path where this artifact is stored.
    pub cache_path: PathBuf,
}

/// Result of ensuring a single plugin artifact is available.
#[derive(Debug, Clone)]
pub struct PluginArtifactResult {
    /// The parent plugin name.
    pub plugin_name: String,
    /// Human-readable artifact label.
    pub artifact_name: String,
    /// Whether this is a compiler plugin or runtime library.
    pub kind: PluginArtifactKind,
    /// Path to the artifact on disk.
    pub path: PathBuf,
    /// Hex-encoded SHA-256 hash.
    pub sha256: String,
    /// Full download URL.
    pub url: String,
    /// Whether the artifact was freshly downloaded.
    pub freshly_downloaded: bool,
}

// ---------------------------------------------------------------------------
// Descriptor loading
// ---------------------------------------------------------------------------

const SERIALIZATION_DESCRIPTOR: &str = include_str!("../../../plugins/serialization.toml");

/// Load all built-in plugin descriptors.
///
/// # Errors
/// Returns an error if any embedded descriptor fails to parse.
pub fn load_descriptors() -> Result<Vec<PluginDescriptor>, EngineError> {
    let sources = [("serialization.toml", SERIALIZATION_DESCRIPTOR)];
    let mut descriptors = Vec::with_capacity(sources.len());

    for (filename, content) in sources {
        let descriptor: PluginDescriptor =
            toml::from_str(content).map_err(|e| EngineError::InvalidPluginConfig {
                name: filename.to_owned(),
                reason: e.to_string(),
            })?;
        descriptors.push(descriptor);
    }

    Ok(descriptors)
}

/// Return a comma-separated list of available plugin names.
fn available_plugin_names(descriptors: &[PluginDescriptor]) -> String {
    descriptors
        .iter()
        .map(|d| d.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// Template substitution
// ---------------------------------------------------------------------------

/// Substitute `{kotlin_version}`, `{version}`, and `{target}` in a template string.
fn substitute_template(
    template: &str,
    kotlin_version: &str,
    plugin_version: &str,
    target_suffix: &str,
) -> String {
    template
        .replace("{kotlin_version}", kotlin_version)
        .replace("{version}", plugin_version)
        .replace("{target}", target_suffix)
}

// ---------------------------------------------------------------------------
// Module dependency resolution
// ---------------------------------------------------------------------------

/// Expand the set of selected module names to include transitive `depends_on`.
///
/// Starting from `always = true` modules plus user-selected modules, iteratively
/// adds any modules listed in `depends_on` until no new modules are discovered.
fn resolve_module_set(
    modules: &BTreeMap<String, ModuleSpec>,
    user_selected: &[String],
) -> BTreeSet<String> {
    let mut selected = BTreeSet::new();

    // Start with always-included modules.
    for (name, spec) in modules {
        if spec.always {
            selected.insert(name.clone());
        }
    }

    // Add user-selected modules.
    for name in user_selected {
        selected.insert(name.clone());
    }

    // Expand transitive dependencies (fixpoint loop).
    loop {
        let mut added = Vec::new();
        for name in &selected {
            if let Some(spec) = modules.get(name) {
                for dep in &spec.depends_on {
                    if !selected.contains(dep) {
                        added.push(dep.clone());
                    }
                }
            }
        }
        if added.is_empty() {
            break;
        }
        for name in added {
            selected.insert(name);
        }
    }

    selected
}

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

/// Return the Maven cache root directory (`~/.konvoy/cache/maven`).
///
/// # Errors
/// Returns an error if the home directory cannot be determined.
fn maven_cache_root() -> Result<PathBuf, EngineError> {
    Ok(konvoy_util::fs::konvoy_home()?.join("cache").join("maven"))
}

/// Resolve all plugin artifacts required by the manifest.
///
/// For each plugin declared in `manifest.plugins`, this finds the matching
/// built-in descriptor, substitutes template variables, parses Maven coordinates,
/// and computes download URLs and cache paths.
///
/// # Errors
/// Returns an error if a plugin name is unknown, a module name is invalid,
/// a Maven coordinate template cannot be parsed, or the cache root cannot be
/// determined.
pub fn resolve_plugin_artifacts(
    manifest: &Manifest,
    target: &Target,
) -> Result<Vec<ResolvedPluginArtifact>, EngineError> {
    let descriptors = load_descriptors()?;
    let cache_root = maven_cache_root()?;
    let kotlin_version = &manifest.toolchain.kotlin;
    let target_suffix = target.to_maven_suffix();

    let mut artifacts = Vec::new();

    for (plugin_name, plugin_config) in &manifest.plugins {
        let descriptor = descriptors
            .iter()
            .find(|d| d.name == *plugin_name)
            .ok_or_else(|| EngineError::UnknownPlugin {
                name: plugin_name.clone(),
                available: available_plugin_names(&descriptors),
            })?;

        // Validate user-selected modules against the descriptor.
        for module_name in &plugin_config.modules {
            if !descriptor.modules.contains_key(module_name) {
                let available_modules: Vec<&str> =
                    descriptor.modules.keys().map(String::as_str).collect();
                return Err(EngineError::UnknownPluginModule {
                    plugin: plugin_name.clone(),
                    module: module_name.clone(),
                    available: available_modules.join(", "),
                });
            }
        }

        // 1. Resolve the compiler plugin JAR.
        let compiler_coord_str = substitute_template(
            &descriptor.compiler_plugin,
            kotlin_version,
            &plugin_config.version,
            &target_suffix,
        );
        let compiler_coord = MavenCoordinate::parse(&compiler_coord_str).map_err(|e| {
            EngineError::InvalidPluginConfig {
                name: plugin_name.clone(),
                reason: format!("invalid compiler plugin coordinate: {e}"),
            }
        })?;
        let compiler_url = compiler_coord.to_url(MAVEN_CENTRAL);
        let compiler_cache = compiler_coord.cache_path(&cache_root);

        artifacts.push(ResolvedPluginArtifact {
            plugin_name: plugin_name.clone(),
            artifact_name: "compiler-plugin".to_owned(),
            kind: PluginArtifactKind::CompilerPlugin,
            maven_coord: compiler_coord,
            url: compiler_url,
            cache_path: compiler_cache,
        });

        // 2. Resolve runtime modules.
        let selected_modules = resolve_module_set(&descriptor.modules, &plugin_config.modules);

        for module_name in &selected_modules {
            let module_spec = descriptor.modules.get(module_name).ok_or_else(|| {
                EngineError::UnknownPluginModule {
                    plugin: plugin_name.clone(),
                    module: module_name.clone(),
                    available: descriptor
                        .modules
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                }
            })?;

            let module_coord_str = substitute_template(
                &module_spec.maven,
                kotlin_version,
                &plugin_config.version,
                &target_suffix,
            );
            let module_coord = MavenCoordinate::parse(&module_coord_str)
                .map_err(|e| EngineError::InvalidPluginConfig {
                    name: plugin_name.clone(),
                    reason: format!("invalid module `{module_name}` coordinate: {e}"),
                })?
                .with_packaging("klib");
            let module_url = module_coord.to_url(MAVEN_CENTRAL);
            let module_cache = module_coord.cache_path(&cache_root);

            artifacts.push(ResolvedPluginArtifact {
                plugin_name: plugin_name.clone(),
                artifact_name: module_name.clone(),
                kind: PluginArtifactKind::Runtime,
                maven_coord: module_coord,
                url: module_url,
                cache_path: module_cache,
            });
        }
    }

    Ok(artifacts)
}

// ---------------------------------------------------------------------------
// Download / verification
// ---------------------------------------------------------------------------

/// Look up the expected SHA-256 for a plugin artifact from the lockfile.
fn find_lockfile_hash<'a>(
    lockfile: &'a Lockfile,
    plugin_name: &str,
    artifact_name: &str,
) -> Option<&'a str> {
    lockfile
        .plugins
        .iter()
        .find(|p| p.name == plugin_name && p.artifact == artifact_name)
        .map(|p| p.sha256.as_str())
}

/// Map a `UtilError::Download` to `EngineError::PluginDownload`.
fn map_download_err(name: &str, e: konvoy_util::error::UtilError) -> EngineError {
    match e {
        konvoy_util::error::UtilError::Download { message } => EngineError::PluginDownload {
            name: name.to_owned(),
            message,
        },
        konvoy_util::error::UtilError::ArtifactHashMismatch {
            expected, actual, ..
        } => EngineError::PluginHashMismatch {
            name: name.to_owned(),
            artifact: name.to_owned(),
            expected,
            actual,
        },
        other => EngineError::Util(other),
    }
}

/// Ensure all plugin artifacts are downloaded, hash-verified, and return results.
///
/// In `--locked` mode, every artifact must already have a hash in the lockfile.
///
/// # Errors
/// Returns an error if an artifact is missing from the lockfile in locked mode,
/// if a download fails, or if a hash does not match.
pub fn ensure_plugin_artifacts(
    artifacts: &[ResolvedPluginArtifact],
    lockfile: &Lockfile,
    locked: bool,
) -> Result<Vec<PluginArtifactResult>, EngineError> {
    let mut results = Vec::with_capacity(artifacts.len());

    for artifact in artifacts {
        let expected_hash =
            find_lockfile_hash(lockfile, &artifact.plugin_name, &artifact.artifact_name);

        // In --locked mode, the hash must be present in the lockfile.
        if locked && expected_hash.is_none() {
            return Err(EngineError::LockfileUpdateRequired);
        }

        let label = format!("{}:{}", artifact.plugin_name, artifact.artifact_name);

        let util_result = konvoy_util::artifact::ensure_artifact(
            &artifact.url,
            &artifact.cache_path,
            expected_hash,
            &label,
            &artifact.maven_coord.version,
        )
        .map_err(|e| map_download_err(&artifact.plugin_name, e))?;

        results.push(PluginArtifactResult {
            plugin_name: artifact.plugin_name.clone(),
            artifact_name: artifact.artifact_name.clone(),
            kind: artifact.kind.clone(),
            path: util_result.path,
            sha256: util_result.sha256,
            url: artifact.url.clone(),
            freshly_downloaded: util_result.freshly_downloaded,
        });
    }

    Ok(results)
}

/// Build `PluginLock` entries from download results.
pub fn build_plugin_locks(results: &[PluginArtifactResult]) -> Vec<PluginLock> {
    results
        .iter()
        .map(|r| {
            let kind_str = match r.kind {
                PluginArtifactKind::CompilerPlugin => "compiler-plugin",
                PluginArtifactKind::Runtime => "runtime",
            };
            PluginLock {
                name: r.plugin_name.clone(),
                artifact: r.artifact_name.clone(),
                kind: kind_str.to_owned(),
                sha256: r.sha256.clone(),
                url: r.url.clone(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn load_descriptors_succeeds() {
        let descriptors = load_descriptors().unwrap();
        assert!(!descriptors.is_empty(), "expected at least one descriptor");
        let serialization = descriptors
            .iter()
            .find(|d| d.name == "serialization")
            .unwrap();
        assert!(!serialization.compiler_plugin.is_empty());
        assert!(
            serialization.modules.contains_key("core"),
            "missing core module"
        );
        assert!(
            serialization.modules.contains_key("json"),
            "missing json module"
        );
    }

    #[test]
    fn serialization_descriptor_core_is_always() {
        let descriptors = load_descriptors().unwrap();
        let serialization = descriptors
            .iter()
            .find(|d| d.name == "serialization")
            .unwrap();
        let core = serialization.modules.get("core").unwrap();
        assert!(core.always, "core module should have always = true");
    }

    #[test]
    fn serialization_descriptor_json_depends_on_core() {
        let descriptors = load_descriptors().unwrap();
        let serialization = descriptors
            .iter()
            .find(|d| d.name == "serialization")
            .unwrap();
        let json = serialization.modules.get("json").unwrap();
        assert!(
            json.depends_on.contains(&"core".to_owned()),
            "json should depend on core"
        );
    }

    #[test]
    fn substitute_template_all_placeholders() {
        let template = "org.jetbrains.kotlinx:kotlinx-serialization-core-{target}:{version}";
        let result = substitute_template(template, "2.1.0", "1.8.0", "linuxx64");
        assert_eq!(
            result,
            "org.jetbrains.kotlinx:kotlinx-serialization-core-linuxx64:1.8.0"
        );
    }

    #[test]
    fn substitute_template_kotlin_version() {
        let template = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin:{kotlin_version}";
        let result = substitute_template(template, "2.1.0", "1.8.0", "linuxx64");
        assert_eq!(
            result,
            "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin:2.1.0"
        );
    }

    #[test]
    fn resolve_module_set_always_included() {
        let mut modules = BTreeMap::new();
        modules.insert(
            "core".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: true,
                depends_on: vec![],
            },
        );
        modules.insert(
            "json".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: false,
                depends_on: vec!["core".to_owned()],
            },
        );

        // No user selection — only "core" (always) should be included.
        let selected = resolve_module_set(&modules, &[]);
        assert!(selected.contains("core"));
        assert!(!selected.contains("json"));
    }

    #[test]
    fn resolve_module_set_user_selected_with_transitive() {
        let mut modules = BTreeMap::new();
        modules.insert(
            "core".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: true,
                depends_on: vec![],
            },
        );
        modules.insert(
            "json".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: false,
                depends_on: vec!["core".to_owned()],
            },
        );
        modules.insert(
            "cbor".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: false,
                depends_on: vec!["core".to_owned()],
            },
        );

        let selected = resolve_module_set(&modules, &["json".to_owned()]);
        assert!(
            selected.contains("core"),
            "core should be included (always)"
        );
        assert!(
            selected.contains("json"),
            "json should be included (user-selected)"
        );
        assert!(!selected.contains("cbor"), "cbor should not be included");
    }

    #[test]
    fn resolve_module_set_transitive_chain() {
        let mut modules = BTreeMap::new();
        modules.insert(
            "a".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: false,
                depends_on: vec![],
            },
        );
        modules.insert(
            "b".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: false,
                depends_on: vec!["a".to_owned()],
            },
        );
        modules.insert(
            "c".to_owned(),
            ModuleSpec {
                maven: "unused".to_owned(),
                always: false,
                depends_on: vec!["b".to_owned()],
            },
        );

        // Selecting "c" should transitively pull in "b" and "a".
        let selected = resolve_module_set(&modules, &["c".to_owned()]);
        assert!(selected.contains("a"));
        assert!(selected.contains("b"));
        assert!(selected.contains("c"));
    }

    #[test]
    fn resolve_plugin_artifacts_unknown_plugin() {
        let manifest = make_manifest_with_plugin("unknown-plugin", "1.0.0", &[]);
        let target = Target::from_str("linux_x64").unwrap();
        let result = resolve_plugin_artifacts(&manifest, &target);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown plugin"), "error was: {err}");
        assert!(
            err.contains("serialization"),
            "error should list available plugins: {err}"
        );
    }

    #[test]
    fn resolve_plugin_artifacts_unknown_module() {
        let manifest = make_manifest_with_plugin("serialization", "1.8.0", &["nonexistent"]);
        let target = Target::from_str("linux_x64").unwrap();
        let result = resolve_plugin_artifacts(&manifest, &target);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unknown module"), "error was: {err}");
    }

    #[test]
    fn resolve_plugin_artifacts_serialization_basic() {
        let manifest = make_manifest_with_plugin("serialization", "1.8.0", &[]);
        let target = Target::from_str("linux_x64").unwrap();
        let artifacts = resolve_plugin_artifacts(&manifest, &target).unwrap();

        // Should have: 1 compiler plugin + 1 core (always).
        assert_eq!(artifacts.len(), 2);

        let compiler = artifacts
            .iter()
            .find(|a| a.kind == PluginArtifactKind::CompilerPlugin)
            .unwrap();
        assert!(
            compiler
                .url
                .contains("kotlin-serialization-compiler-plugin"),
            "url was: {}",
            compiler.url
        );
        assert!(
            compiler.url.contains("2.1.0"),
            "url should contain kotlin version: {}",
            compiler.url
        );

        let core = artifacts
            .iter()
            .find(|a| a.artifact_name == "core")
            .unwrap();
        assert_eq!(core.kind, PluginArtifactKind::Runtime);
        assert!(
            core.url.contains("kotlinx-serialization-core-linuxx64"),
            "url was: {}",
            core.url
        );
        assert!(
            core.url.contains("1.8.0"),
            "url should contain plugin version: {}",
            core.url
        );
        // Runtime klibs should use .klib packaging.
        assert!(
            core.cache_path.display().to_string().ends_with(".klib"),
            "cache_path was: {}",
            core.cache_path.display()
        );
    }

    #[test]
    fn resolve_plugin_artifacts_with_json_module() {
        let manifest = make_manifest_with_plugin("serialization", "1.8.0", &["json"]);
        let target = Target::from_str("linux_x64").unwrap();
        let artifacts = resolve_plugin_artifacts(&manifest, &target).unwrap();

        // compiler-plugin + core (always) + json (user-selected).
        assert_eq!(artifacts.len(), 3);
        assert!(artifacts.iter().any(|a| a.artifact_name == "json"));
        assert!(artifacts.iter().any(|a| a.artifact_name == "core"));
    }

    #[test]
    fn resolve_plugin_artifacts_macos_target() {
        let manifest = make_manifest_with_plugin("serialization", "1.8.0", &[]);
        let target = Target::from_str("macos_arm64").unwrap();
        let artifacts = resolve_plugin_artifacts(&manifest, &target).unwrap();

        let core = artifacts
            .iter()
            .find(|a| a.artifact_name == "core")
            .unwrap();
        assert!(
            core.url.contains("macosarm64"),
            "url should use macos target suffix: {}",
            core.url
        );
    }

    #[test]
    fn maven_coord_generation_compiler_plugin() {
        let template = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin:{kotlin_version}";
        let coord_str = substitute_template(template, "2.1.0", "1.8.0", "linuxx64");
        let coord = MavenCoordinate::parse(&coord_str).unwrap();
        assert_eq!(coord.group_id, "org.jetbrains.kotlin");
        assert_eq!(coord.artifact_id, "kotlin-serialization-compiler-plugin");
        assert_eq!(coord.version, "2.1.0");
        assert_eq!(coord.packaging, "jar");
    }

    #[test]
    fn maven_coord_generation_runtime_module() {
        let template = "org.jetbrains.kotlinx:kotlinx-serialization-core-{target}:{version}";
        let coord_str = substitute_template(template, "2.1.0", "1.8.0", "linuxx64");
        let coord = MavenCoordinate::parse(&coord_str)
            .unwrap()
            .with_packaging("klib");
        assert_eq!(coord.group_id, "org.jetbrains.kotlinx");
        assert_eq!(coord.artifact_id, "kotlinx-serialization-core-linuxx64");
        assert_eq!(coord.version, "1.8.0");
        assert_eq!(coord.packaging, "klib");
    }

    #[test]
    fn cache_path_uses_maven_layout() {
        let manifest = make_manifest_with_plugin("serialization", "1.8.0", &[]);
        let target = Target::from_str("linux_x64").unwrap();
        let artifacts = resolve_plugin_artifacts(&manifest, &target).unwrap();

        let compiler = artifacts
            .iter()
            .find(|a| a.kind == PluginArtifactKind::CompilerPlugin)
            .unwrap();

        let path_str = compiler.cache_path.display().to_string();
        assert!(
            path_str.contains("cache/maven"),
            "path should be under cache/maven: {path_str}"
        );
        assert!(
            path_str.contains("org/jetbrains/kotlin"),
            "path should have group path: {path_str}"
        );
    }

    #[test]
    fn build_plugin_locks_from_results() {
        let results = vec![
            PluginArtifactResult {
                plugin_name: "serialization".to_owned(),
                artifact_name: "compiler-plugin".to_owned(),
                kind: PluginArtifactKind::CompilerPlugin,
                path: PathBuf::from("/cache/plugin.jar"),
                sha256: "abc123".to_owned(),
                url: "https://example.com/plugin.jar".to_owned(),
                freshly_downloaded: true,
            },
            PluginArtifactResult {
                plugin_name: "serialization".to_owned(),
                artifact_name: "core".to_owned(),
                kind: PluginArtifactKind::Runtime,
                path: PathBuf::from("/cache/core.klib"),
                sha256: "def456".to_owned(),
                url: "https://example.com/core.klib".to_owned(),
                freshly_downloaded: false,
            },
        ];

        let locks = build_plugin_locks(&results);
        assert_eq!(locks.len(), 2);
        assert_eq!(
            locks.first().map(|l| l.kind.as_str()),
            Some("compiler-plugin")
        );
        assert_eq!(locks.get(1).map(|l| l.kind.as_str()), Some("runtime"));
    }

    #[test]
    fn find_lockfile_hash_present() {
        let lockfile = Lockfile {
            plugins: vec![PluginLock {
                name: "serialization".to_owned(),
                artifact: "compiler-plugin".to_owned(),
                kind: "compiler-plugin".to_owned(),
                sha256: "abc123".to_owned(),
                url: "https://example.com".to_owned(),
            }],
            ..Lockfile::default()
        };

        let hash = find_lockfile_hash(&lockfile, "serialization", "compiler-plugin");
        assert_eq!(hash, Some("abc123"));
    }

    #[test]
    fn find_lockfile_hash_absent() {
        let lockfile = Lockfile::default();
        let hash = find_lockfile_hash(&lockfile, "serialization", "compiler-plugin");
        assert!(hash.is_none());
    }

    #[test]
    fn available_plugin_names_format() {
        let descriptors = load_descriptors().unwrap();
        let names = available_plugin_names(&descriptors);
        assert!(names.contains("serialization"), "names was: {names}");
    }

    // -- Helper -----------------------------------------------------------------

    use std::str::FromStr;

    fn make_manifest_with_plugin(plugin_name: &str, version: &str, modules: &[&str]) -> Manifest {
        let mut plugins = BTreeMap::new();
        plugins.insert(
            plugin_name.to_owned(),
            konvoy_config::manifest::PluginConfig {
                version: version.to_owned(),
                modules: modules.iter().map(|m| (*m).to_owned()).collect(),
            },
        );
        Manifest {
            package: konvoy_config::manifest::Package {
                name: "test-project".to_owned(),
                kind: konvoy_config::manifest::PackageKind::Bin,
                version: None,
                entrypoint: "src/main.kt".to_owned(),
            },
            toolchain: konvoy_config::manifest::Toolchain {
                kotlin: "2.1.0".to_owned(),
                detekt: None,
            },
            dependencies: BTreeMap::new(),
            plugins,
        }
    }
}
