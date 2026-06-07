use std::fs;

use konvoy_config::lockfile::Lockfile;
use konvoy_config::manifest::{Codegen, OpenApiCodegen};
use konvoy_engine::codegen::ManagedToolSpec;
use konvoy_util::maven::MavenCoordinate;

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
        }),
    };
    let second = Codegen {
        openapi: Some(OpenApiCodegen {
            version: "20.0.0".to_owned(),
            spec: "openapi.yaml".to_owned(),
            base_package: "com.example.second".to_owned(),
        }),
    };

    let first_hashes = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &first).unwrap();
    let second_hashes =
        konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &second).unwrap();

    assert_ne!(first_hashes, second_hashes);
}

fn openapi_codegen(version: &str, spec: &str, base_package: &str) -> Codegen {
    Codegen {
        openapi: Some(OpenApiCodegen {
            version: version.to_owned(),
            spec: spec.to_owned(),
            base_package: base_package.to_owned(),
        }),
    }
}

#[test]
fn openapi_codegen_hash_changes_when_spec_content_changes() {
    let dir = tempfile::tempdir().unwrap();
    let spec = dir.path().join("openapi.yaml");
    let codegen = openapi_codegen("20.0.0", "openapi.yaml", "com.example.api");

    fs::write(&spec, "openapi: 3.1.0\ninfo:\n  title: A\n  version: 1.0.0\n").unwrap();
    let before = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    // Same path, different bytes — the cache key MUST change.
    fs::write(&spec, "openapi: 3.1.0\ninfo:\n  title: B\n  version: 2.0.0\n").unwrap();
    let after = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    assert_ne!(before, after, "editing the spec content must change the hash");
}

#[test]
fn openapi_codegen_hash_tracks_transitive_ref_files() {
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

    let codegen = openapi_codegen("20.0.0", "openapi.yaml", "com.example.api");
    let before = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    // Changing ONLY the referenced sub-spec must still change the hash.
    fs::write(
        components.join("pet.yaml"),
        "Pet:\n  type: object\n  properties:\n    id:\n      type: string\n",
    )
    .unwrap();
    let after = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen).unwrap();

    assert_ne!(
        before, after,
        "editing a $ref-ed sub-spec must change the codegen hash"
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
fn run_codegen_locked_without_pin_requires_lockfile_update() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("openapi.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\n",
    )
    .unwrap();

    let version = format!("99.0.0-locked-nopin-{}", std::process::id());
    let codegen = openapi_codegen(&version, "openapi.yaml", "com.example");

    let lockfile_path = dir.path().join("konvoy.lock");
    // Lockfile has a toolchain but NO codegen tool pin.
    let lockfile = Lockfile::with_toolchain("2.1.0");

    let result = konvoy_engine::codegen::run_codegen(
        dir.path(),
        &codegen,
        &lockfile,
        &lockfile_path,
        "2.1.0",
        None,
        false,
        true, // locked
        false,
    );

    assert!(matches!(
        result,
        Err(konvoy_engine::EngineError::LockfileUpdateRequired)
    ));
}

#[test]
fn run_codegen_locked_with_pin_but_missing_jar_reports_tool_not_found() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("openapi.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\n",
    )
    .unwrap();

    let version = format!("99.0.0-locked-missingjar-{}", std::process::id());
    // Ensure the JAR is genuinely absent.
    let jar = konvoy_engine::codegen::openapi::fabrikt_jar_path(&version).unwrap();
    let _ = fs::remove_file(&jar);

    let codegen = openapi_codegen(&version, "openapi.yaml", "com.example");

    let lockfile_path = dir.path().join("konvoy.lock");
    let mut lockfile = Lockfile::with_toolchain("2.1.0");
    // Matching pin exists, but the artifact is not downloaded.
    lockfile.set_codegen_tool("fabrikt", &version, "deadbeef");

    let result = konvoy_engine::codegen::run_codegen(
        dir.path(),
        &codegen,
        &lockfile,
        &lockfile_path,
        "2.1.0",
        None,
        false,
        true, // locked
        false,
    );

    assert!(
        matches!(
            result,
            Err(konvoy_engine::EngineError::CodegenToolNotFound { ref name, .. }) if name == "fabrikt"
        ),
        "expected CodegenToolNotFound, got {result:?}"
    );
}

#[test]
fn run_codegen_replaces_stale_generic_tool_pin_without_regenerating_sources() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("openapi.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: Demo\n  version: 1.0.0\n",
    )
    .unwrap();

    let version = format!("99.0.0-test-pin-{}", std::process::id());
    let jar = konvoy_engine::codegen::openapi::fabrikt_jar_path(&version).unwrap();
    if let Some(parent) = jar.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&jar, b"fake fabrikt jar").unwrap();
    let real_hash = konvoy_util::hash::sha256_file(&jar).unwrap();

    let codegen = Codegen {
        openapi: Some(OpenApiCodegen {
            version: version.clone(),
            spec: "openapi.yaml".to_owned(),
            base_package: "com.example".to_owned(),
        }),
    };
    let tagged_hash = konvoy_engine::codegen::compute_codegen_hashes(dir.path(), &codegen)
        .unwrap()
        .pop()
        .unwrap();
    let input_hash = tagged_hash
        .strip_prefix("openapi:")
        .unwrap_or_else(|| panic!("unexpected hash tag: {tagged_hash}"));
    let output_dir = konvoy_engine::codegen::generator_output_dir(dir.path(), "openapi");
    fs::create_dir_all(&output_dir).unwrap();
    fs::write(output_dir.join(".input_hash"), format!("{input_hash}\n")).unwrap();

    let lockfile_path = dir.path().join("konvoy.lock");
    let mut lockfile = Lockfile::with_toolchain("2.1.0");
    lockfile.set_codegen_tool("fabrikt", "old-version", "old-hash");
    lockfile.write_to(&lockfile_path).unwrap();
    let lockfile = Lockfile::from_path(&lockfile_path).unwrap();

    let generated = konvoy_engine::codegen::run_codegen(
        dir.path(),
        &codegen,
        &lockfile,
        &lockfile_path,
        "2.1.0",
        None,
        false,
        false,
        false,
    )
    .unwrap();

    assert!(generated.is_empty());
    let updated = Lockfile::from_path(&lockfile_path).unwrap();
    let pin = updated
        .codegen_tool("fabrikt")
        .unwrap_or_else(|| panic!("missing updated fabrikt pin"));
    assert_eq!(pin.version, version);
    assert_eq!(pin.sha256, real_hash);

    let _ = fs::remove_file(&jar);
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
fn ensure_fabrikt_hash_mismatch_on_existing_jar() {
    let version = format!("99.0.0-test-mismatch-{}", std::process::id());
    let jar = konvoy_engine::codegen::openapi::fabrikt_jar_path(&version).unwrap();
    if let Some(parent) = jar.parent() {
        fs::create_dir_all(parent).unwrap();
    }
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
    fs::write(&jar, b"already cached").unwrap();
    let real_hash = konvoy_util::hash::sha256_file(&jar).unwrap();

    let result = konvoy_engine::codegen::openapi::ensure_fabrikt(&version, Some(&real_hash));

    assert!(result.is_ok(), "matching hash should pass: {result:?}");
    let (path, hash) = result.unwrap();
    assert_eq!(path, jar);
    assert_eq!(hash, real_hash);
    let _ = fs::remove_file(&path);
}
