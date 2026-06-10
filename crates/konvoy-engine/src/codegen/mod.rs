//! Declarative source generation before Kotlin/Native compilation.
//!
//! This module is the **generator-agnostic** framework: the [`CodeGenerator`]
//! trait plus the deterministic input hashing that folds each generator's config
//! and input files into the build cache key. It knows nothing about any concrete
//! generator (OpenAPI/Fabrikt, gRPC, …) — those implement [`CodeGenerator`] and
//! are assembled into a `&[Box<dyn CodeGenerator>]` by a registry that lives with
//! the implementations, so adding a generator never touches this file.

use std::path::{Component, Path, PathBuf};

use crate::error::EngineError;

// The managed-tool abstraction is shared with the detekt linter, so it lives at
// the engine root (`crate::managed_tool`); re-exported here for codegen callers.
pub use crate::managed_tool::ManagedToolSpec;

/// Display metadata for a configured generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratorSummary {
    /// Stable generator name used in paths and cache key tags.
    pub name: String,
    /// Human-readable generator label.
    pub display_name: String,
    /// Directory containing this generator's outputs.
    pub output_dir: PathBuf,
}

/// A configured code generator.
///
/// Implemented by each concrete generator (OpenAPI/Fabrikt today; gRPC etc.
/// later). The framework only ever sees `dyn CodeGenerator`, so a new generator
/// is added by implementing this trait and registering it — never by editing the
/// framework.
pub trait CodeGenerator {
    /// Stable generator name used in paths and cache key tags.
    fn name(&self) -> &str;

    /// Human-readable generator label.
    fn display_name(&self) -> &str;

    /// Managed tool required by this generator.
    fn managed_tool(&self) -> ManagedToolSpec;

    /// Stable config fields that affect generated sources.
    fn config_hash_parts(&self) -> Vec<String>;

    /// Project-relative input files read by this generator.
    ///
    /// `project_root` lets generators enumerate inputs that live on disk (e.g.
    /// every file under a configured spec directory). Paths should be
    /// **project-relative** (so the cache key is portable across machines); their
    /// contents are folded into the generator hash, and thus the build cache key.
    /// A generator need not bother with order, duplicates, `./` prefixes, or even
    /// absolute paths under `project_root`: the framework normalizes, sorts, and
    /// de-duplicates before hashing (see `normalized_input_key`).
    ///
    /// # Errors
    /// Returns an error if a configured input location (e.g. a spec directory)
    /// is missing or cannot be read.
    fn input_files(&self, project_root: &Path) -> Result<Vec<PathBuf>, EngineError>;

    /// Generate sources into `output_dir`.
    ///
    /// The tool returned by [`managed_tool`](Self::managed_tool) must already be
    /// downloaded; the generator runs it through
    /// [`ManagedToolSpec::run`](crate::managed_tool::ManagedToolSpec::run). `jre_home`
    /// is the managed JRE for JVM generators and `None` for a native generator —
    /// it is forwarded to `run` per the tool's runtime.
    ///
    /// # Errors
    /// Returns an error if the generator process cannot be executed or fails.
    fn generate(
        &self,
        project_root: &Path,
        output_dir: &Path,
        jre_home: Option<&Path>,
        verbose: bool,
    ) -> Result<(), EngineError>;
}

/// Return display summaries for the given generators.
#[must_use]
pub fn generator_summaries(
    project_root: &Path,
    generators: &[Box<dyn CodeGenerator>],
) -> Vec<GeneratorSummary> {
    generators
        .iter()
        .map(|generator| GeneratorSummary {
            name: generator.name().to_owned(),
            display_name: generator.display_name().to_owned(),
            output_dir: generator_output_dir(project_root, generator.name()),
        })
        .collect()
}

/// Return the managed tools required by the given generators.
#[must_use]
pub fn managed_tools(generators: &[Box<dyn CodeGenerator>]) -> Vec<ManagedToolSpec> {
    generators
        .iter()
        .map(|generator| generator.managed_tool())
        .collect()
}

/// Compute `(generator name, input hash)` pairs for the given generators, in the
/// order provided.
///
/// This is the single place generator hashes are computed: a build computes them
/// once for the cache key and can thread them into generation so neither the hash
/// nor any input file is read a second time on a cache-miss build.
///
/// # Errors
/// Returns an error if a configured generator input cannot be read.
pub fn compute_codegen_hash_pairs(
    project_root: &Path,
    generators: &[Box<dyn CodeGenerator>],
) -> Result<Vec<(String, String)>, EngineError> {
    generators
        .iter()
        .map(|generator| {
            let hash = compute_generator_hash(generator.as_ref(), project_root)?;
            Ok((generator.name().to_owned(), hash))
        })
        .collect()
}

/// Compute tagged (`name:hash`) hashes for the given generators — the form folded
/// into the build cache key.
///
/// # Errors
/// Returns an error if a configured generator input cannot be read.
pub fn compute_codegen_hashes(
    project_root: &Path,
    generators: &[Box<dyn CodeGenerator>],
) -> Result<Vec<String>, EngineError> {
    Ok(compute_codegen_hash_pairs(project_root, generators)?
        .into_iter()
        .map(|(name, hash)| format!("{name}:{hash}"))
        .collect())
}

/// Return the output directory for a generator under `.konvoy/gen/`.
#[must_use]
pub fn generator_output_dir(project_root: &Path, name: &str) -> PathBuf {
    project_root.join(".konvoy").join("gen").join(name)
}

fn compute_generator_hash(
    generator: &dyn CodeGenerator,
    project_root: &Path,
) -> Result<String, EngineError> {
    // `display_name` is deliberately NOT hashed: it's a cosmetic label that does
    // not affect generated output, so a future rename must not invalidate every
    // project's codegen cache. `name` IS hashed — it picks the output dir and tags
    // the key, and distinguishes two generators with otherwise-identical inputs.
    let mut parts = vec!["codegen-v1".to_owned(), generator.name().to_owned()];
    parts.extend(generator.config_hash_parts());

    // The framework owns cache-key determinism. Normalize each declared input to a
    // stable, project-relative key (see `normalized_input_key`), then sort and
    // de-duplicate so the key never depends on input order, accidental duplicates,
    // spelling variants of the same path (`./a` vs `a`), or a machine-specific
    // absolute prefix — so generators need not normalize.
    let mut keys: Vec<PathBuf> = generator
        .input_files(project_root)?
        .iter()
        .map(|input| normalized_input_key(project_root, input))
        .collect();
    keys.sort();
    keys.dedup();

    // Mark the boundary between the config section and the per-file section, so a
    // config part can never be mistaken for (or collide with) a file triple.
    parts.push("inputs".to_owned());
    for key in keys {
        // The key must be a safe, project-relative file path. Reject anything that
        // would make the cache key non-portable or read outside the project: an
        // empty key (an input of `.` / the root itself), an absolute path not under
        // `project_root` (would embed a machine-specific prefix), or a `..` escape.
        // A generator violating the documented contract is a bug, surfaced loudly
        // here rather than silently producing a mis-keyed or unsafe build.
        if key.as_os_str().is_empty()
            || key.is_absolute()
            || key.components().any(|c| matches!(c, Component::ParentDir))
        {
            return Err(EngineError::InternalInvariantViolated {
                context: format!(
                    "generator `{}` returned an input that is not a project-relative file: {}",
                    generator.name(),
                    key.display()
                ),
            });
        }
        let full_path = project_root.join(&key);
        // Hash the path's raw bytes (not Display, which is lossy for non-UTF-8
        // names) so a rename always changes the key and the key is encoding-stable.
        parts.push("file".to_owned());
        parts.push(konvoy_util::hash::sha256_bytes(
            key.as_os_str().as_encoded_bytes(),
        ));
        // Read content directly (no exists() pre-check): that races with the read
        // and reports EACCES as "not found". Map only a genuine NotFound to the
        // actionable codegen error; surface other I/O errors (e.g. permission) as-is.
        match konvoy_util::hash::sha256_file(&full_path) {
            Ok(hash) => parts.push(hash),
            Err(konvoy_util::error::UtilError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Err(EngineError::CodegenInputNotFound {
                    name: generator.name().to_owned(),
                    path: full_path.display().to_string(),
                });
            }
            Err(e) => return Err(EngineError::from(e)),
        }
    }

    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    Ok(konvoy_util::hash::sha256_multi(&refs))
}

/// Canonicalize a generator's declared input path into a stable, project-relative
/// key for hashing.
///
/// - Strips the `project_root` prefix if the generator returned an absolute path
///   under it, so the key carries no machine-specific prefix (portable cache key).
/// - Drops redundant `.` / separator components (lexical only — never touches the
///   filesystem, so it stays portable), so spelling variants of the same file
///   (`./a.yaml` vs `a.yaml`) collapse to one key when de-duplicated.
///
/// `..` is preserved here (lexical normalization must not resolve it), as is an
/// absolute path not under `project_root`. Both are contract violations
/// (`input_files` must return project-relative paths); the caller
/// ([`compute_generator_hash`]) rejects such keys rather than silently baking a
/// non-portable or out-of-project path into the cache key.
fn normalized_input_key(project_root: &Path, input: &Path) -> PathBuf {
    input
        .strip_prefix(project_root)
        .unwrap_or(input)
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal generator used to exercise the framework's hashing with NO
    /// dependency on any real generator — proving the core is decoupled. `name` and
    /// `display_name` are separate so tests can vary them independently.
    struct FakeGenerator {
        name: String,
        display_name: String,
        config_parts: Vec<String>,
        inputs: Vec<PathBuf>,
    }

    impl CodeGenerator for FakeGenerator {
        fn name(&self) -> &str {
            &self.name
        }
        fn display_name(&self) -> &str {
            &self.display_name
        }
        fn managed_tool(&self) -> ManagedToolSpec {
            ManagedToolSpec::direct_url(
                &self.name,
                &self.display_name,
                "1.0.0",
                "https://example.invalid/fake.jar".to_owned(),
                "fake-1.0.0.jar".to_owned(),
            )
        }
        fn config_hash_parts(&self) -> Vec<String> {
            self.config_parts.clone()
        }
        fn input_files(&self, _project_root: &Path) -> Result<Vec<PathBuf>, EngineError> {
            Ok(self.inputs.clone())
        }
        fn generate(
            &self,
            _project_root: &Path,
            _output_dir: &Path,
            _jre_home: Option<&Path>,
            _verbose: bool,
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    /// Build a fake generator (display_name defaults to name).
    fn fake(name: &str, config_parts: &[&str], inputs: &[&str]) -> Box<dyn CodeGenerator> {
        fake_full(name, name, config_parts, inputs)
    }

    /// Build a fake generator with an explicit display_name.
    fn fake_full(
        name: &str,
        display_name: &str,
        config_parts: &[&str],
        inputs: &[&str],
    ) -> Box<dyn CodeGenerator> {
        Box::new(FakeGenerator {
            name: name.to_owned(),
            display_name: display_name.to_owned(),
            config_parts: config_parts.iter().map(|s| (*s).to_owned()).collect(),
            inputs: inputs.iter().map(PathBuf::from).collect(),
        })
    }

    /// Single tagged hash for one generator (most tests use exactly one).
    fn hash_one(root: &Path, generator: Box<dyn CodeGenerator>) -> String {
        let mut out = compute_codegen_hashes(root, &[generator]).unwrap();
        assert_eq!(out.len(), 1);
        out.remove(0)
    }

    // ---- shape / determinism ------------------------------------------------

    #[test]
    fn hashes_are_tagged_and_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
        let gens = vec![fake("demo", &["v=1"], &["a.txt"])];

        let first = compute_codegen_hashes(tmp.path(), &gens).unwrap();
        let second = compute_codegen_hashes(tmp.path(), &gens).unwrap();
        assert_eq!(first, second, "hashing must be deterministic");
        assert_eq!(first.len(), 1);
        assert!(
            first[0].starts_with("demo:"),
            "hash must be tagged name:hash, got {}",
            first[0]
        );
    }

    #[test]
    fn hash_is_stable_across_project_roots() {
        // Same relative inputs + content under two different roots must hash equal
        // (the key must not depend on the absolute project path).
        let make = || {
            let tmp = tempfile::tempdir().unwrap();
            std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
            let h = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
            (tmp, h)
        };
        let (_t1, h1) = make();
        let (_t2, h2) = make();
        assert_eq!(h1, h2);
    }

    // ---- what MUST change the hash -----------------------------------------

    #[test]
    fn hash_changes_when_an_input_file_changes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
        let before = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));

        std::fs::write(tmp.path().join("a.txt"), "beta").unwrap();
        let after = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
        assert_ne!(before, after, "editing an input must change the hash");
    }

    #[test]
    fn hash_changes_when_config_changes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
        let a = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
        let b = hash_one(tmp.path(), fake("demo", &["v=2"], &["a.txt"]));
        assert_ne!(a, b, "a config change must change the hash");
    }

    #[test]
    fn hash_changes_when_an_input_is_renamed() {
        // Same content at a different path must change the key — the input path
        // (not just its content) is part of the hash.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "same").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "same").unwrap();
        let a = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
        let b = hash_one(tmp.path(), fake("demo", &["v=1"], &["b.txt"]));
        assert_ne!(a, b, "a path change (rename) must change the hash");
    }

    #[test]
    fn distinct_names_produce_distinct_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
        let a = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
        let b = hash_one(tmp.path(), fake("other", &["v=1"], &["a.txt"]));
        assert_ne!(a, b, "the generator name must distinguish the key");
    }

    // ---- what must NOT change the hash (fixes #1, #2) -----------------------

    #[test]
    fn hash_ignores_display_name() {
        // display_name is a cosmetic label and must not affect the cache key —
        // otherwise renaming it would needlessly invalidate every project's cache.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "alpha").unwrap();
        let a = hash_one(
            tmp.path(),
            fake_full("demo", "OpenAPI", &["v=1"], &["a.txt"]),
        );
        let b = hash_one(
            tmp.path(),
            fake_full("demo", "Renamed Label", &["v=1"], &["a.txt"]),
        );
        assert_eq!(a, b, "display_name must not be part of the cache key");
    }

    #[test]
    fn hash_is_insensitive_to_input_order() {
        // The framework sorts inputs, so the order a generator returns them in must
        // not affect the key (a future generator forgetting to sort is still safe).
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "A").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "B").unwrap();
        let ab = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt", "b.txt"]));
        let ba = hash_one(tmp.path(), fake("demo", &["v=1"], &["b.txt", "a.txt"]));
        assert_eq!(ab, ba, "input order must not affect the hash");
    }

    #[test]
    fn duplicate_inputs_do_not_change_hash() {
        // The framework de-duplicates, so an accidentally-repeated input is a no-op.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "A").unwrap();
        let once = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
        let twice = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt", "a.txt"]));
        assert_eq!(once, twice, "duplicate inputs must not change the hash");
    }

    #[test]
    fn path_spelling_variants_collapse_to_one_input() {
        // `./a.txt` and `a.txt` name the same file; the framework normalizes (drops
        // `.`) so they de-duplicate rather than hashing the file twice under two keys.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "A").unwrap();
        let plain = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
        let dotted = hash_one(
            tmp.path(),
            fake("demo", &["v=1"], &["./a.txt", "a.txt", "./a.txt"]),
        );
        assert_eq!(
            plain, dotted,
            "`./a.txt` must normalize + dedup against `a.txt`"
        );
    }

    #[test]
    fn absolute_input_under_root_matches_relative() {
        // A generator that (against the contract) returns an absolute path under the
        // project root must still yield a portable, machine-independent key: the
        // framework strips the root prefix, so it hashes identically to the relative
        // form. Otherwise the same project would hash differently on another machine.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "A").unwrap();
        let relative = hash_one(tmp.path(), fake("demo", &["v=1"], &["a.txt"]));
        let absolute = hash_one(
            tmp.path(),
            Box::new(FakeGenerator {
                name: "demo".to_owned(),
                display_name: "demo".to_owned(),
                config_parts: vec!["v=1".to_owned()],
                inputs: vec![tmp.path().join("a.txt")], // absolute, under root
            }),
        );
        assert_eq!(
            relative, absolute,
            "an absolute-under-root input must hash like the relative form"
        );
    }

    #[test]
    fn hash_format_is_stable_v1() {
        // Golden hash pinning the FULL codegen-v1 wire format — including the per-file
        // section (the "file" marker, the path-key hash, the content hash, and their
        // order), which an input-free generator would NOT exercise. The input path is
        // project-relative and the content fixed, so the digest is deterministic
        // across machines despite the tempdir root. If it changes, the cache-key
        // format changed — every project's codegen cache will invalidate, so bump the
        // `codegen-v1` tag deliberately and update this value (run the test; copy the
        // `left` digest from the failure).
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("api.yaml"), "openapi: 3.1.0").unwrap();
        let gen = fake_full(
            "openapi",
            "Display Label Is Not Hashed",
            &["tool_version=1.0.0", "base_package=com.example"],
            &["api.yaml"],
        );
        let hashes = compute_codegen_hashes(tmp.path(), &[gen]).unwrap();
        assert_eq!(
            hashes,
            vec![
                "openapi:1f226a3c437445cccbe5853cce06f94f12d20b3349fd4cadda5f6cafb0bd3da4"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn hash_format_is_stable_v1_multi_file() {
        // Companion golden with TWO input files, pinning the per-file *repetition*
        // layout (the sorted sequence of `file`/path/content triples) — which the
        // single-file golden cannot exercise. A refactor that changed how multiple
        // files are laid out (e.g. all paths then all contents) would keep the
        // single-file golden + order/dedup tests green but break this one.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("specs")).unwrap();
        std::fs::write(tmp.path().join("specs/a.yaml"), "AAA").unwrap();
        std::fs::write(tmp.path().join("specs/b.yaml"), "BBB").unwrap();
        let gen = fake_full(
            "openapi",
            "Display Label Is Not Hashed",
            &["tool_version=1.0.0", "base_package=com.example"],
            &["specs/b.yaml", "specs/a.yaml"], // unsorted on purpose
        );
        let hashes = compute_codegen_hashes(tmp.path(), &[gen]).unwrap();
        assert_eq!(
            hashes,
            vec![
                "openapi:c8791cc0aef7a65d7ab357eef6933aa0760ad8662c0524aa9a2abbd494f0c7a0"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn rejects_inputs_that_are_not_project_relative_files() {
        // Contract enforcement: an input of `.` (empty key → would hash the project
        // dir), an absolute path outside the root (would embed a machine-specific
        // prefix), or a `..` escape (would read outside the project) is a loud
        // InternalInvariantViolated, never a silently mis-keyed or unsafe build.
        let tmp = tempfile::tempdir().unwrap();
        for bad in [".", "/etc/passwd", "../secret.txt"] {
            match compute_codegen_hashes(tmp.path(), &[fake("demo", &["v=1"], &[bad])]) {
                Err(EngineError::InternalInvariantViolated { .. }) => {}
                other => panic!("input {bad:?} must be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn parent_dir_escape_is_rejected_without_reading_the_target() {
        // The `..` case above uses a non-existent path; this proves the guard blocks
        // the read of a file that ACTUALLY EXISTS outside the project root (not just
        // that a missing file errors). Nest the root so `../secret.txt` resolves to a
        // real file outside it: were the ParentDir guard reverted, that file would be
        // read and the call would return Ok — so requiring InternalInvariantViolated
        // proves the escape is blocked before any read.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("secret.txt"), "outside the project").unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();

        match compute_codegen_hashes(&root, &[fake("demo", &["v=1"], &["../secret.txt"])]) {
            Err(EngineError::InternalInvariantViolated { .. }) => {}
            other => panic!("a `..` escape to an existing file must be rejected, got {other:?}"),
        }
    }

    // ---- edge cases ---------------------------------------------------------

    #[test]
    fn generator_with_no_inputs_hashes_only_config() {
        // A generator that declares no input files still hashes (over its config),
        // deterministically, and tracks config changes.
        let tmp = tempfile::tempdir().unwrap();
        let a = hash_one(tmp.path(), fake("demo", &["v=1"], &[]));
        let a2 = hash_one(tmp.path(), fake("demo", &["v=1"], &[]));
        let b = hash_one(tmp.path(), fake("demo", &["v=2"], &[]));
        assert_eq!(a, a2, "no-input hashing must be deterministic");
        assert_ne!(a, b, "config still feeds the hash with no inputs");
    }

    #[test]
    fn missing_input_file_is_codegen_input_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let gens = vec![fake("demo", &["v=1"], &["does-not-exist.txt"])];
        match compute_codegen_hashes(tmp.path(), &gens) {
            Err(EngineError::CodegenInputNotFound { name, path }) => {
                assert_eq!(name, "demo");
                assert!(path.ends_with("does-not-exist.txt"), "got {path}");
            }
            other => panic!("expected CodegenInputNotFound, got {other:?}"),
        }
    }

    // ---- multiple generators / API consistency ------------------------------

    #[test]
    fn multiple_generators_are_hashed_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "A").unwrap();
        let gens = vec![
            fake("aaa", &["v=1"], &["a.txt"]),
            fake("bbb", &["v=1"], &["a.txt"]),
        ];
        let hashes = compute_codegen_hashes(tmp.path(), &gens).unwrap();
        assert_eq!(hashes.len(), 2);
        assert!(hashes[0].starts_with("aaa:"));
        assert!(hashes[1].starts_with("bbb:"));
    }

    #[test]
    fn empty_generator_list_produces_no_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(compute_codegen_hashes(tmp.path(), &[]).unwrap().is_empty());
        assert!(compute_codegen_hash_pairs(tmp.path(), &[])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn tagged_hashes_match_pairs() {
        // compute_codegen_hashes must be exactly `name:hash` of the pairs form.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "A").unwrap();
        let gens = vec![fake("demo", &["v=1"], &["a.txt"])];
        let pairs = compute_codegen_hash_pairs(tmp.path(), &gens).unwrap();
        let tagged = compute_codegen_hashes(tmp.path(), &gens).unwrap();
        let expected: Vec<String> = pairs
            .iter()
            .map(|(name, hash)| format!("{name}:{hash}"))
            .collect();
        assert_eq!(tagged, expected);
    }

    // ---- read-side helpers --------------------------------------------------

    #[test]
    fn output_dir_is_under_dot_konvoy_gen() {
        let dir = generator_output_dir(Path::new("/proj"), "openapi");
        assert_eq!(dir, PathBuf::from("/proj/.konvoy/gen/openapi"));
    }

    #[test]
    fn summaries_and_managed_tools_reflect_the_given_generators() {
        let gens = vec![
            fake_full("demo", "Demo Label", &[], &[]),
            fake("other", &[], &[]),
        ];
        let summaries = generator_summaries(Path::new("/proj"), &gens);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].name, "demo");
        assert_eq!(summaries[0].display_name, "Demo Label");
        assert_eq!(
            summaries[0].output_dir,
            PathBuf::from("/proj/.konvoy/gen/demo")
        );
        assert_eq!(summaries[1].name, "other");
        assert_eq!(managed_tools(&gens).len(), 2);
    }
}
