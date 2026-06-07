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
