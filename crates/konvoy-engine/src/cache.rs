//! Content-addressed cache key computation for build artifacts.

use std::fmt;
use std::path::Path;

use crate::error::EngineError;

/// All inputs that contribute to a deterministic cache key.
#[derive(Debug)]
pub struct CacheInputs {
    /// Normalized konvoy.toml content.
    pub manifest_content: String,
    /// Relevant konvoy.lock content (toolchain version).
    pub lockfile_content: String,
    /// konanc version string.
    pub konanc_version: String,
    /// SHA-256 fingerprint of the konanc binary.
    pub konanc_fingerprint: String,
    /// Target triple (e.g. "linux_x64").
    pub target: String,
    /// Build profile ("debug" or "release").
    pub profile: String,
    /// Root directory containing source files.
    pub source_dir: std::path::PathBuf,
    /// Glob pattern for source files (e.g. "**/*.kt").
    pub source_glob: String,
    /// Operating system identifier.
    pub os: String,
    /// Architecture identifier.
    pub arch: String,
    /// SHA-256 hashes of dependency `.klib` files (empty for projects with no deps).
    pub dependency_hashes: Vec<String>,
}

impl CacheInputs {
    /// Create inputs with OS and arch auto-detected from the current platform.
    pub fn with_platform_defaults(mut self) -> Self {
        self.os = std::env::consts::OS.to_owned();
        self.arch = std::env::consts::ARCH.to_owned();
        self
    }
}

/// A content-addressed cache key wrapping a SHA-256 hex string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(String);

impl CacheKey {
    /// Compute a cache key from the given inputs.
    ///
    /// Source files are sorted by path before hashing for determinism.
    /// Only file contents are hashed, not metadata like timestamps.
    ///
    /// # Errors
    /// Returns an error if source files cannot be read.
    pub fn compute(inputs: &CacheInputs) -> Result<Self, EngineError> {
        let source_hash = konvoy_util::hash::sha256_dir(&inputs.source_dir, &inputs.source_glob)?;

        let mut parts: Vec<&str> = vec![
            &inputs.manifest_content,
            &inputs.lockfile_content,
            &inputs.konanc_version,
            &inputs.konanc_fingerprint,
            &inputs.target,
            &inputs.profile,
            &source_hash,
            &inputs.os,
            &inputs.arch,
        ];
        // Include dependency hashes so cache key changes when deps are rebuilt.
        for h in &inputs.dependency_hashes {
            parts.push(h);
        }

        let composite = konvoy_util::hash::sha256_multi(&parts);

        Ok(Self(composite))
    }

    /// Return the hex string representation of this cache key.
    pub fn as_hex(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<Path> for CacheKey {
    fn as_ref(&self) -> &Path {
        Path::new(&self.0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;
    use proptest::prelude::*;

    fn make_inputs(dir: &Path) -> CacheInputs {
        CacheInputs {
            manifest_content: "[package]\nname = \"test\"".to_owned(),
            lockfile_content: "".to_owned(),
            konanc_version: "2.1.0".to_owned(),
            konanc_fingerprint: "abc123".to_owned(),
            target: "linux_x64".to_owned(),
            profile: "debug".to_owned(),
            source_dir: dir.to_path_buf(),
            source_glob: "**/*.kt".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            dependency_hashes: Vec::new(),
        }
    }

    fn setup_sources(dir: &Path) {
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("main.kt"), "fun main() {}").unwrap();
    }

    #[test]
    fn same_inputs_same_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();
        let key2 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();
        assert_eq!(key1, key2);
    }

    #[test]
    fn key_has_valid_hex_length() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key = CacheKey::compute(&make_inputs(tmp.path())).unwrap();
        assert_eq!(key.as_hex().len(), 64);
    }

    #[test]
    fn display_matches_hex() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key = CacheKey::compute(&make_inputs(tmp.path())).unwrap();
        assert_eq!(key.to_string(), key.as_hex());
    }

    #[test]
    fn changing_manifest_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.manifest_content = "[package]\nname = \"other\"".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn changing_source_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        fs::write(
            tmp.path().join("src").join("main.kt"),
            "fun main() { changed }",
        )
        .unwrap();
        let key2 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn changing_target_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.target = "macos_arm64".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn changing_profile_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.profile = "release".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn changing_konanc_version_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.konanc_version = "2.2.0".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn changing_konanc_fingerprint_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.konanc_fingerprint = "xyz789".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn adding_source_file_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        fs::write(tmp.path().join("src").join("extra.kt"), "fun extra() {}").unwrap();
        let key2 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn file_order_does_not_affect_key() {
        let dir1 = tempfile::tempdir().unwrap();
        let src1 = dir1.path().join("src");
        fs::create_dir_all(&src1).unwrap();
        fs::write(src1.join("b.kt"), "fun b() {}").unwrap();
        fs::write(src1.join("a.kt"), "fun a() {}").unwrap();

        let dir2 = tempfile::tempdir().unwrap();
        let src2 = dir2.path().join("src");
        fs::create_dir_all(&src2).unwrap();
        fs::write(src2.join("a.kt"), "fun a() {}").unwrap();
        fs::write(src2.join("b.kt"), "fun b() {}").unwrap();

        let mut inputs1 = make_inputs(dir1.path());
        inputs1.source_dir = dir1.path().to_path_buf();
        let key1 = CacheKey::compute(&inputs1).unwrap();

        let mut inputs2 = make_inputs(dir2.path());
        inputs2.source_dir = dir2.path().to_path_buf();
        let key2 = CacheKey::compute(&inputs2).unwrap();

        assert_eq!(key1, key2);
    }

    #[test]
    fn changing_os_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.os = "macos".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn changing_arch_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.arch = "aarch64".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn changing_lockfile_changes_key() {
        let tmp = tempfile::tempdir().unwrap();
        setup_sources(tmp.path());

        let key1 = CacheKey::compute(&make_inputs(tmp.path())).unwrap();

        let mut inputs = make_inputs(tmp.path());
        inputs.lockfile_content = "[toolchain]\nkonanc_version = \"2.0.0\"".to_owned();
        let key2 = CacheKey::compute(&inputs).unwrap();

        assert_ne!(key1, key2);
    }

    proptest! {
        #[test]
        fn same_inputs_always_produce_same_key(
            manifest in "\\PC{0,100}",
            lockfile in "\\PC{0,100}",
            version in "\\PC{1,20}",
            fingerprint in "\\PC{1,20}",
            target in "\\PC{1,20}",
            profile in "\\PC{1,20}",
            os in "\\PC{1,20}",
            arch in "\\PC{1,20}",
        ) {
            let tmp = tempfile::tempdir().unwrap();
            setup_sources(tmp.path());

            let inputs_a = CacheInputs {
                manifest_content: manifest.clone(),
                lockfile_content: lockfile.clone(),
                konanc_version: version.clone(),
                konanc_fingerprint: fingerprint.clone(),
                target: target.clone(),
                profile: profile.clone(),
                source_dir: tmp.path().to_path_buf(),
                source_glob: "**/*.kt".to_owned(),
                os: os.clone(),
                arch: arch.clone(),
                dependency_hashes: Vec::new(),
            };
            let inputs_b = CacheInputs {
                manifest_content: manifest,
                lockfile_content: lockfile,
                konanc_version: version,
                konanc_fingerprint: fingerprint,
                target,
                profile,
                source_dir: tmp.path().to_path_buf(),
                source_glob: "**/*.kt".to_owned(),
                os,
                arch,
                dependency_hashes: Vec::new(),
            };

            let key_a = CacheKey::compute(&inputs_a).unwrap();
            let key_b = CacheKey::compute(&inputs_b).unwrap();
            prop_assert_eq!(key_a, key_b);
        }
    }
}
