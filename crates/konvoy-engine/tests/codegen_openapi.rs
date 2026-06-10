use std::fs;

use konvoy_config::manifest::{Codegen, OpenApiCodegen};
use konvoy_engine::codegen::ManagedToolSpec;
use konvoy_util::maven::MavenCoordinate;

/// Removes a managed-tool version directory on drop, so tests that write fake
/// JARs under the real `~/.konvoy/tools/` don't leave junk behind — even when an
/// assertion panics mid-test.
struct CleanupDir(std::path::PathBuf);
impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn managed_tool_spec_formats_maven_jar_url_and_path() {
    let spec = ManagedToolSpec::maven_jar(
        "grpc",
        "gRPC Kotlin generator",
        MavenCoordinate::new("io.grpc", "protoc-gen-grpc-kotlin", "1.4.1"),
    );

    assert_eq!(
        spec.download_url(),
        "https://repo1.maven.org/maven2/io/grpc/protoc-gen-grpc-kotlin/1.4.1/protoc-gen-grpc-kotlin-1.4.1.jar"
    );
    let path = spec.artifact_path().unwrap();
    let rendered = path.display().to_string();
    assert!(
        rendered.contains(".konvoy/tools/grpc/1.4.1"),
        "path was: {rendered}"
    );
    assert!(
        rendered.contains("protoc-gen-grpc-kotlin-1.4.1.jar"),
        "path was: {rendered}"
    );
}

#[test]
fn openapi_codegen_hash_includes_generator_config() {
    let dir = tempfile::tempdir().unwrap();
    let spec = dir.path().join("openapi.yaml");
    fs::write(
        &spec,
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\n",
    )
    .unwrap();

    let first = Codegen {
        openapi: Some(OpenApiCodegen {
            version: "20.0.0".to_owned(),
            spec: "openapi.yaml".to_owned(),
            base_package: "com.example.first".to_owned(),
            extra_spec_dirs: Vec::new(),
        }),
    };
    let second = Codegen {
        openapi: Some(OpenApiCodegen {
            version: "20.0.0".to_owned(),
            spec: "openapi.yaml".to_owned(),
            base_package: "com.example.second".to_owned(),
            extra_spec_dirs: Vec::new(),
        }),
    };

    let first_hashes = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &first).unwrap();
    let second_hashes =
        konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &second).unwrap();

    assert_ne!(first_hashes, second_hashes);
}

fn openapi_codegen(version: &str, spec: &str, base_package: &str) -> Codegen {
    openapi_codegen_with_dirs(version, spec, base_package, &[])
}

fn openapi_codegen_with_dirs(
    version: &str,
    spec: &str,
    base_package: &str,
    extra_spec_dirs: &[&str],
) -> Codegen {
    Codegen {
        openapi: Some(OpenApiCodegen {
            version: version.to_owned(),
            spec: spec.to_owned(),
            base_package: base_package.to_owned(),
            extra_spec_dirs: extra_spec_dirs.iter().map(|d| (*d).to_owned()).collect(),
        }),
    }
}

#[test]
fn openapi_codegen_hash_changes_when_spec_content_changes() {
    let dir = tempfile::tempdir().unwrap();
    let spec = dir.path().join("openapi.yaml");
    let codegen = openapi_codegen("20.0.0", "openapi.yaml", "com.example.api");

    fs::write(
        &spec,
        "openapi: 3.1.0\ninfo:\n  title: A\n  version: 1.0.0\n",
    )
    .unwrap();
    let before = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    // Same path, different bytes — the cache key MUST change.
    fs::write(
        &spec,
        "openapi: 3.1.0\ninfo:\n  title: B\n  version: 2.0.0\n",
    )
    .unwrap();
    let after = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    assert_ne!(
        before, after,
        "editing the spec content must change the hash"
    );
}

#[test]
fn openapi_codegen_hash_tracks_files_under_extra_spec_dirs() {
    // Fabrikt resolves `$ref`'d sibling files internally but never reports them,
    // so Konvoy tracks them by directory: listing the sub-spec's directory in
    // `extra_spec_dirs` makes a change to ANY file under it invalidate the hash.
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components");
    fs::create_dir_all(&components).unwrap();
    fs::write(
        dir.path().join("openapi.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\ncomponents:\n  schemas:\n    Pet:\n      $ref: './components/pet.yaml#/Pet'\n",
    )
    .unwrap();
    fs::write(
        components.join("pet.yaml"),
        "Pet:\n  type: object\n  properties:\n    id:\n      type: integer\n",
    )
    .unwrap();

    let codegen =
        openapi_codegen_with_dirs("20.0.0", "openapi.yaml", "com.example.api", &["components"]);
    let before = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    // Changing ONLY the referenced sub-spec must change the hash.
    fs::write(
        components.join("pet.yaml"),
        "Pet:\n  type: object\n  properties:\n    id:\n      type: string\n",
    )
    .unwrap();
    let after = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    assert_ne!(
        before, after,
        "editing a file under a spec_dir must change the codegen hash"
    );
}

#[test]
fn openapi_codegen_hash_ignores_siblings_without_extra_spec_dirs() {
    // The flip side of the accepted tradeoff: without `extra_spec_dirs`, only the
    // primary spec is tracked. A change to a sibling `$ref`'d file is NOT picked
    // up — users must opt in by listing the directory. This documents (and locks)
    // that Konvoy does not parse the spec to discover `$ref` targets.
    let dir = tempfile::tempdir().unwrap();
    let components = dir.path().join("components");
    fs::create_dir_all(&components).unwrap();
    fs::write(
        dir.path().join("openapi.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\ncomponents:\n  schemas:\n    Pet:\n      $ref: './components/pet.yaml#/Pet'\n",
    )
    .unwrap();
    fs::write(components.join("pet.yaml"), "Pet:\n  type: object\n").unwrap();

    let codegen = openapi_codegen("20.0.0", "openapi.yaml", "com.example.api");
    let before = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    fs::write(
        components.join("pet.yaml"),
        "Pet:\n  type: object\n  properties:\n    id:\n      type: string\n",
    )
    .unwrap();
    let after = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    assert_eq!(
        before, after,
        "without extra_spec_dirs, a sibling file change must not affect the hash"
    );
}

#[test]
fn openapi_codegen_hash_errors_when_spec_dir_missing() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("openapi.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\n",
    )
    .unwrap();
    let codegen = openapi_codegen_with_dirs(
        "20.0.0",
        "openapi.yaml",
        "com.example.api",
        &["does-not-exist"],
    );

    let result = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen);
    assert!(
        matches!(
            result,
            Err(konvoy_engine::EngineError::CodegenInputDirNotFound { .. })
        ),
        "expected CodegenInputDirNotFound, got {result:?}"
    );
}

#[test]
fn openapi_codegen_hash_is_stable_across_runs_and_project_roots() {
    let codegen = openapi_codegen("20.0.0", "openapi.yaml", "com.example.api");
    let spec_bytes = "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\n";

    let dir1 = tempfile::tempdir().unwrap();
    fs::write(dir1.path().join("openapi.yaml"), spec_bytes).unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    fs::write(dir2.path().join("openapi.yaml"), spec_bytes).unwrap();

    let h1a = konvoy_engine::codegen::compute_codegen_hashes(dir1.path(), &codegen).unwrap();
    let h1b = konvoy_engine::codegen::compute_codegen_hashes(dir1.path(), &codegen).unwrap();
    let h2 = konvoy_engine::codegen::compute_codegen_hashes(dir2.path(), &codegen).unwrap();

    assert_eq!(h1a, h1b, "identical inputs must produce identical hashes");
    assert_eq!(
        h1a, h2,
        "the hash must be independent of the absolute project location"
    );
}

#[test]
fn openapi_codegen_hash_errors_when_spec_missing() {
    let dir = tempfile::tempdir().unwrap();
    let codegen = openapi_codegen("20.0.0", "missing.yaml", "com.example.api");

    let result = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen);
    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("missing.yaml"),
        "error should name the missing spec, was: {message}"
    );
}

#[test]
fn codegen_hash_pairs_match_tagged_hashes() {
    // compute_codegen_hashes (cache-key form) must be exactly the tagged
    // rendering of compute_codegen_hash_pairs, so threading the pairs into the
    // build does not change the cache key bytes.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("openapi.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\n",
    )
    .unwrap();
    let codegen = openapi_codegen("20.0.0", "openapi.yaml", "com.example.api");

    let pairs = konvoy_engine::codegen::compute_codegen_hash_pairs(dir.path(), &codegen).unwrap();
    let tagged = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    let from_pairs: Vec<String> = pairs.iter().map(|(n, h)| format!("{n}:{h}")).collect();
    assert_eq!(tagged, from_pairs);
    assert!(pairs.iter().any(|(name, _)| name == "openapi"));
}

#[test]
fn fabrikt_download_url_format() {
    let url = konvoy_engine::codegen::openapi::fabrikt_download_url("20.0.0");
    assert_eq!(
        url,
        "https://repo1.maven.org/maven2/com/cjbooms/fabrikt/20.0.0/fabrikt-20.0.0.jar"
    );
}

#[test]
fn fabrikt_jar_path_format() {
    let path = konvoy_engine::codegen::openapi::fabrikt_jar_path("20.0.0").unwrap();
    let rendered = path.display().to_string();
    assert!(
        rendered.contains(".konvoy/tools/fabrikt/20.0.0"),
        "path was: {rendered}"
    );
    assert!(
        rendered.contains("fabrikt-20.0.0.jar"),
        "path was: {rendered}"
    );
}

#[test]
fn ensure_fabrikt_rejects_invalid_version() {
    let result = konvoy_engine::codegen::openapi::ensure_fabrikt("../bad", None);
    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("invalid fabrikt version"),
        "error was: {message}"
    );
}

#[test]
fn ensure_fabrikt_rejects_dotdot_version() {
    // Charset-valid but contains `..` (e.g. a `1..2` typo): the weaker
    // validate_version would have let this through, so this is the regression guard
    // for the validate_identifier fix. `../bad` above does NOT exercise it (the `/`
    // is rejected by either check).
    let result = konvoy_engine::codegen::openapi::ensure_fabrikt("1..2", None);
    let message = result
        .expect_err("a `..` version must be rejected")
        .to_string();
    assert!(
        message.contains("invalid fabrikt version"),
        "expected the curated fabrikt message, got: {message}"
    );
}

#[test]
fn ensure_fabrikt_hash_mismatch_on_existing_jar() {
    let version = format!("99.0.0-test-mismatch-{}", std::process::id());
    let jar = konvoy_engine::codegen::openapi::fabrikt_jar_path(&version).unwrap();
    if let Some(parent) = jar.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let _cleanup = CleanupDir(
        jar.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default(),
    );
    fs::write(&jar, b"not the expected jar").unwrap();

    let result = konvoy_engine::codegen::openapi::ensure_fabrikt(
        &version,
        Some("0000000000000000000000000000000000000000000000000000000000000000"),
    );

    assert!(result.is_err());
    let message = result.unwrap_err().to_string();
    assert!(message.contains("hash mismatch"), "error was: {message}");
    let _ = fs::remove_file(&jar);
}

#[test]
fn ensure_fabrikt_accepts_matching_hash_on_existing_jar() {
    let version = format!("99.0.0-test-match-{}", std::process::id());
    let jar = konvoy_engine::codegen::openapi::fabrikt_jar_path(&version).unwrap();
    if let Some(parent) = jar.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let _cleanup = CleanupDir(
        jar.parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default(),
    );
    fs::write(&jar, b"already cached").unwrap();
    let real_hash = konvoy_util::hash::sha256_file(&jar).unwrap();

    let result = konvoy_engine::codegen::openapi::ensure_fabrikt(&version, Some(&real_hash));

    assert!(result.is_ok(), "matching hash should pass: {result:?}");
    let (path, hash) = result.unwrap();
    assert_eq!(path, jar);
    assert_eq!(hash, real_hash);
    let _ = fs::remove_file(&path);
}
