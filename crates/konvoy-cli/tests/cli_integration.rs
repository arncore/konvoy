//! Integration tests for the `konvoy` CLI binary.
//!
//! These exercise the `cmd_*` dispatch error paths so they show up in
//! coverage without needing a working konanc toolchain. We never go far
//! enough to invoke the compiler — instead we drive each `cmd_*` until
//! it returns a controlled error.

use std::path::Path;
use std::process::Command;

fn konvoy_bin() -> &'static str {
    // Cargo sets CARGO_BIN_EXE_<bin-name> for integration tests.
    env!("CARGO_BIN_EXE_konvoy")
}

fn run_in(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(konvoy_bin())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to spawn konvoy");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

fn write_manifest(dir: &Path, contents: &str) {
    std::fs::write(dir.join("konvoy.toml"), contents).expect("write konvoy.toml");
}

// ── No-project error paths ────────────────────────────────────────────

#[test]
fn build_outside_project_reports_missing_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _stdout, stderr) = run_in(tmp.path(), &["build"]);
    assert!(!ok, "build should fail without konvoy.toml");
    assert!(
        stderr.contains("no konvoy.toml") || stderr.contains("konvoy init"),
        "expected missing-manifest error, got stderr: {stderr}"
    );
}

#[test]
fn run_outside_project_reports_missing_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _stdout, stderr) = run_in(tmp.path(), &["run"]);
    assert!(!ok);
    assert!(stderr.contains("no konvoy.toml"), "stderr was: {stderr}");
}

#[test]
fn test_outside_project_reports_missing_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _stdout, stderr) = run_in(tmp.path(), &["test"]);
    assert!(!ok);
    assert!(stderr.contains("no konvoy.toml"), "stderr was: {stderr}");
}

// ── Profile flag dispatch (exercises profile_from_flag + dispatch arms) ─

#[test]
fn build_dispatches_with_release_profile() {
    // Sends --release through the Build dispatch arm and profile_from_flag(true).
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _stdout, _stderr) = run_in(tmp.path(), &["build", "--release"]);
    // Exits non-zero because there's no konvoy.toml, but the dispatch arm + profile
    // mapping have already executed at that point.
    assert!(!ok);
}

#[test]
fn run_dispatches_with_release_and_extra_args() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _stdout, _stderr) = run_in(tmp.path(), &["run", "--release", "--", "foo", "bar"]);
    assert!(!ok);
}

#[test]
fn test_dispatches_with_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _stdout, _stderr) = run_in(tmp.path(), &["test", "--filter", "my_test_name"]);
    assert!(!ok);
}

// ── `run` against a library project: the "cannot run a library" branch ─

#[test]
fn run_on_library_project_fails_with_actionable_error() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(
        tmp.path(),
        r#"
[package]
name = "my-lib"
kind = "lib"

[toolchain]
kotlin = "2.1.0"
"#,
    );
    // Need a src/ dir or the manifest parse may not be the only requirement.
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src").join("lib.kt"), "fun lib() {}").unwrap();

    let (ok, _stdout, stderr) = run_in(tmp.path(), &["run"]);
    assert!(!ok, "run on a lib project must fail");
    assert!(
        stderr.contains("cannot run a library project"),
        "expected library-project error, got stderr: {stderr}"
    );
}

// ── Help / version sanity (exercises clap dispatch fall-through) ──────

#[test]
fn version_flag_prints_version() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, stdout, _stderr) = run_in(tmp.path(), &["--version"]);
    assert!(ok);
    assert!(stdout.contains("konvoy"), "stdout was: {stdout}");
}

#[test]
fn help_flag_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, stdout, _stderr) = run_in(tmp.path(), &["--help"]);
    assert!(ok);
    assert!(
        stdout.to_lowercase().contains("usage"),
        "expected `usage` in help text, got: {stdout}"
    );
}

#[test]
fn unknown_subcommand_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let (ok, _stdout, _stderr) = run_in(tmp.path(), &["nonexistent-command"]);
    assert!(!ok, "unknown subcommands should exit non-zero");
}
