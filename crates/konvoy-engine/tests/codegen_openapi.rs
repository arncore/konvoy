use std::fs;

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
