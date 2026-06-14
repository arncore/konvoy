//! End-to-end smoke tests for the `--locked` and `--offline` reproducibility
//! flags, driven through the real `konvoy` binary exactly as a user would.
//!
//! ## Approach
//!
//! Each test spawns the compiled `konvoy` binary in a throwaway project
//! directory with `HOME` pointed at a per-test temp dir, so the managed-artifact
//! cache (`~/.konvoy`) is fully isolated — empty by default, or seeded with
//! *fake* artifacts when a test needs one "present". This makes every test
//! hermetic (no network, no shared global state, safe to run in parallel) and
//! lets us assert the precise error a user sees on a clean machine.
//!
//! ## What is and isn't reachable here
//!
//! The build pipeline resolves the toolchain — and execs `konanc` — before it
//! touches compiler plugins or Maven dependency klibs. A fake `konanc` can't
//! actually compile, so:
//!   * The **toolchain**, **maven-dependency-not-resolved**, **lockfile
//!     staleness** (version/plugin drift), and **detekt JAR/JRE** gates all fire
//!     *before* any real compilation and ARE smoke-tested here.
//!   * The **plugin-artifact** (`PluginOffline`) and **library-klib**
//!     (`LibraryOffline`) gates fire *after* `konanc` runs, so they can't be
//!     reached without a real toolchain — those are covered by the engine unit
//!     tests (`ensure_plugin_artifacts_offline_*`, `resolve_maven_deps_offline_*`).
//!
//! For the "the gate lets a present artifact through" cases we stage a fake
//! (non-executable) `konanc`: resolution gets *past* the gate and then fails
//! downstream for an unrelated reason — so those tests assert the failure is NOT
//! the gate error (and, under `--locked`, that `konvoy.lock` was left untouched).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn konvoy_bin() -> &'static str {
    // Cargo sets CARGO_BIN_EXE_<bin-name> for integration tests.
    env!("CARGO_BIN_EXE_konvoy")
}

/// A hermetic project + isolated `~/.konvoy` for one smoke test.
struct Fixture {
    _tmp: TempDir,
    root: PathBuf,
    home: PathBuf,
}

struct Outcome {
    success: bool,
    #[allow(dead_code)]
    stdout: String,
    stderr: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("project");
        let home = tmp.path().join("home");
        fs::create_dir_all(root.join("src")).expect("mkdir project/src");
        fs::create_dir_all(&home).expect("mkdir home");
        // A trivial Kotlin source so the project looks real; the gates we test
        // all fire before sources are ever compiled.
        fs::write(root.join("src").join("main.kt"), "fun main() {}\n").expect("write main.kt");
        Self {
            _tmp: tmp,
            root,
            home,
        }
    }

    fn manifest(&self, contents: &str) -> &Self {
        fs::write(self.root.join("konvoy.toml"), contents).expect("write konvoy.toml");
        self
    }

    fn lockfile(&self, contents: &str) -> &Self {
        fs::write(self.lock_path(), contents).expect("write konvoy.lock");
        self
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join("konvoy.lock")
    }

    fn read_lock(&self) -> Option<String> {
        fs::read_to_string(self.lock_path()).ok()
    }

    /// Stage a *present-but-fake* toolchain so the toolchain gate (and the
    /// `--locked` staleness check) pass. `konanc` is written non-executable, so
    /// resolution fails downstream at `check_executable` with a non-gate error —
    /// which is exactly what the "passes the gate" tests assert against.
    fn stage_toolchain(&self, version: &str) -> &Self {
        let v = self.home.join(".konvoy").join("toolchains").join(version);
        fs::create_dir_all(v.join("bin")).expect("mkdir toolchain/bin");
        fs::write(v.join("bin").join("konanc"), b"not a real konanc\n").expect("write konanc");
        // `is_installed` only checks that the `jre/` directory exists.
        fs::create_dir_all(v.join("jre")).expect("mkdir toolchain/jre");
        self
    }

    /// Stage a present-but-fake detekt CLI JAR at the path the engine derives,
    /// so the detekt JAR gate passes and lint reaches JRE resolution.
    fn stage_detekt_jar(&self, version: &str) -> &Self {
        let dir = self
            .home
            .join(".konvoy")
            .join("tools")
            .join("detekt")
            .join(version);
        fs::create_dir_all(&dir).expect("mkdir tools/detekt/<version>");
        fs::write(
            dir.join(format!("detekt-cli-{version}-all.jar")),
            b"not a real detekt jar\n",
        )
        .expect("write detekt jar");
        self
    }

    fn run(&self, args: &[&str]) -> Outcome {
        let output = Command::new(konvoy_bin())
            .args(args)
            .current_dir(&self.root)
            // Isolate ~/.konvoy. `konvoy_home()` reads HOME (then USERPROFILE).
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .output()
            .expect("spawn konvoy");
        Outcome {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
}

// Manifest/lockfile builders ------------------------------------------------

const KOTLIN: &str = "0.0.0-smoke"; // a version that is never really installed
const DETEKT: &str = "0.0.0-smoke-detekt";
const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn manifest_kotlin(version: &str) -> String {
    format!("[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{version}\"\n")
}

fn manifest_kotlin_detekt(kotlin: &str, detekt: &str) -> String {
    format!(
        "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{kotlin}\"\ndetekt = \"{detekt}\"\n"
    )
}

fn manifest_with_maven_dep(kotlin: &str) -> String {
    format!(
        "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{kotlin}\"\n\n\
         [dependencies]\nsome-lib = {{ maven = \"org.example:some-lib\", version = \"1.0.0\" }}\n"
    )
}

fn manifest_with_plugin(kotlin: &str) -> String {
    format!(
        "[package]\nname = \"demo\"\n\n[toolchain]\nkotlin = \"{kotlin}\"\n\n\
         [plugins]\ndemo-plugin = {{ maven = \"org.example:demo-plugin\", version = \"1.0.0\" }}\n"
    )
}

/// Toolchain entry with the version pinned but NO tarball SHAs (the "unpinned"
/// state that cannot be installed under `--locked`).
fn lock_toolchain(version: &str) -> String {
    format!("[toolchain]\nkonanc_version = \"{version}\"\n")
}

/// Fully pinned toolchain entry (version + both tarball SHAs).
fn lock_toolchain_pinned(version: &str) -> String {
    format!(
        "[toolchain]\nkonanc_version = \"{version}\"\n\
         konanc_tarball_sha256 = \"{SHA}\"\njre_tarball_sha256 = \"{SHA}\"\n"
    )
}

fn assert_lock_unchanged(fixture: &Fixture, before: &Option<String>) {
    assert_eq!(
        before.as_deref(),
        fixture.read_lock().as_deref(),
        "konvoy.lock must not be modified"
    );
}

// ===========================================================================
// --offline : "no network; every managed artifact must already be local"
// ===========================================================================

/// Clean machine, committed lockfile, `konvoy build --offline`, but the pinned
/// toolchain was never fetched → a clear, actionable hard error naming
/// `--offline` and the version. konvoy.lock is left untouched.
#[test]
fn offline_build_errors_when_toolchain_absent() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN));
    let before = f.read_lock();

    let out = f.run(&["build", "--offline"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("--offline"),
        "error should mention --offline: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains(KOTLIN) && out.stderr.contains("not installed"),
        "error should name the missing toolchain: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// With the toolchain already cached, `--offline` must NOT block the build: it
/// gets past the offline gate (and fails later for an unrelated reason, since
/// our staged `konanc` is fake). The point is that no `--offline` error fires.
#[test]
fn offline_build_passes_gate_when_toolchain_present() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN))
        .stage_toolchain(KOTLIN);

    let out = f.run(&["build", "--offline"]);

    assert!(
        !out.stderr.contains("--offline"),
        "a present toolchain must not trip the offline gate: {}",
        out.stderr
    );
}

/// `--offline` must refuse the automatic `konvoy update` (which fetches POMs and
/// klibs from Maven Central) for a manifest Maven dep that isn't in the
/// lockfile — failing fast with the offending dependency named, no network.
#[test]
fn offline_build_refuses_auto_update_for_unresolved_maven_dep() {
    let f = Fixture::new();
    f.manifest(&manifest_with_maven_dep(KOTLIN))
        .lockfile(&lock_toolchain(KOTLIN));
    let before = f.read_lock();

    let out = f.run(&["build", "--offline"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("some-lib") && out.stderr.contains("not resolved"),
        "should name the unresolved dependency: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("konvoy update"),
        "should point the user at `konvoy update`: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// `konvoy lint --offline` with detekt configured but its JAR not cached →
/// `DetektJarOffline`, naming the detekt version and `--offline`.
#[test]
fn offline_lint_errors_when_detekt_jar_absent() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin_detekt(KOTLIN, DETEKT))
        .lockfile(&lock_toolchain(KOTLIN));

    let out = f.run(&["lint", "--offline"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("detekt") && out.stderr.contains(DETEKT),
        "error should name detekt + version: {}",
        out.stderr
    );
    assert!(
        out.stderr.contains("--offline"),
        "error should mention --offline: {}",
        out.stderr
    );
}

/// Regression guard: a `lint --offline` that fails at JRE resolution (detekt JAR
/// is cached, but the toolchain providing the JRE is not) must report
/// `ToolchainJreOffline` AND must NOT leave a rewritten konvoy.lock behind — the
/// detekt-hash persist happens only after the JRE resolves successfully.
#[test]
fn offline_lint_jre_failure_leaves_lockfile_untouched() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin_detekt(KOTLIN, DETEKT))
        .lockfile(&lock_toolchain(KOTLIN)) // toolchain (JRE) NOT staged
        .stage_detekt_jar(DETEKT); // detekt JAR IS present
    let before = f.read_lock();

    let out = f.run(&["lint", "--offline"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("--offline") && out.stderr.contains(KOTLIN),
        "expected a JRE-offline error naming the toolchain: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// The flags are wired through `run` too, not just `build`.
#[test]
fn offline_run_errors_when_toolchain_absent() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN));

    let out = f.run(&["run", "--offline"]);

    assert!(!out.success);
    assert!(
        out.stderr.contains("--offline") && out.stderr.contains(KOTLIN),
        "run --offline should hit the same toolchain gate: {}",
        out.stderr
    );
}

/// ...and through `test`.
#[test]
fn offline_test_errors_when_toolchain_absent() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN));

    let out = f.run(&["test", "--offline"]);

    assert!(!out.success);
    assert!(
        out.stderr.contains("--offline") && out.stderr.contains(KOTLIN),
        "test --offline should hit the same toolchain gate: {}",
        out.stderr
    );
}

// ===========================================================================
// --locked : "reproducible install; never modify konvoy.lock; drift errors"
// ===========================================================================

/// The canonical `--locked` failure: the user bumped `kotlin` in konvoy.toml but
/// forgot to re-run update, so CI's `konvoy build --locked` reports drift. The
/// lockfile is not rewritten.
#[test]
fn locked_build_errors_on_toolchain_version_drift() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin("1.0.0-smoke"))
        .lockfile(&lock_toolchain_pinned("9.9.9-stale"));
    let before = f.read_lock();

    let out = f.run(&["build", "--locked"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("lockfile is out of date") && out.stderr.contains("--locked"),
        "expected lockfile-drift error: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// `--locked` on a clean machine: a version-only toolchain entry (no pinned
/// tarball SHAs) can't be installed reproducibly, so it's drift — fail before
/// any download, and don't touch the lockfile.
#[test]
fn locked_build_errors_when_toolchain_unpinned_and_absent() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin(KOTLIN))
        .lockfile(&lock_toolchain(KOTLIN)); // version matches, but no tarball SHAs
    let before = f.read_lock();

    let out = f.run(&["build", "--locked"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("lockfile is out of date"),
        "expected lockfile-drift error: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// `lint --locked` with detekt configured and its version pinned, but no JAR
/// hash recorded → drift at the detekt JAR gate. Lockfile untouched.
#[test]
fn locked_lint_errors_when_detekt_hash_not_pinned() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin_detekt(KOTLIN, DETEKT))
        .lockfile(
            // detekt VERSION matches the manifest (staleness passes) but the JAR
            // hash is absent, so the JAR pin gate reports drift under --locked.
            &format!("[toolchain]\nkonanc_version = \"{KOTLIN}\"\ndetekt_version = \"{DETEKT}\"\n"),
        );
    let before = f.read_lock();

    let out = f.run(&["lint", "--locked"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("lockfile is out of date"),
        "expected lockfile-drift error: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// `--locked` must NOT silently auto-run update for an unresolved Maven dep;
/// it reports drift instead. Lockfile untouched.
#[test]
fn locked_build_errors_for_unresolved_maven_dep() {
    let f = Fixture::new();
    f.manifest(&manifest_with_maven_dep(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN));
    let before = f.read_lock();

    let out = f.run(&["build", "--locked"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("lockfile is out of date"),
        "expected lockfile-drift error: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// `--locked` with a plugin declared in the manifest but absent from the
/// lockfile → drift, before any toolchain work. Lockfile untouched.
#[test]
fn locked_build_errors_when_plugin_missing_from_lockfile() {
    let f = Fixture::new();
    f.manifest(&manifest_with_plugin(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN)); // no [[plugins]] entry
    let before = f.read_lock();

    let out = f.run(&["build", "--locked"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("lockfile is out of date"),
        "expected lockfile-drift error: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// A complete, consistent, fully-pinned lockfile with the toolchain present:
/// `--locked` must get *past* the staleness check and the toolchain gate (it
/// fails later only because our staged `konanc` is fake) and must leave
/// konvoy.lock byte-for-byte unchanged — the core `--locked` guarantee.
#[test]
fn locked_build_passes_gate_and_preserves_lockfile() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN))
        .stage_toolchain(KOTLIN);
    let before = f.read_lock();

    let out = f.run(&["build", "--locked"]);

    assert!(
        !out.stderr.contains("lockfile is out of date"),
        "a consistent pinned lockfile must not report drift: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

// ===========================================================================
// --locked --offline  (Cargo's --frozen): both at once
// ===========================================================================

/// Under `--frozen`, when the lockfile is BOTH drifting AND the artifact is
/// absent, drift is the actionable root cause and wins — the user sees the
/// lockfile error, not the offline error.
#[test]
fn frozen_reports_drift_before_offline() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin("1.0.0-smoke"))
        .lockfile(&lock_toolchain_pinned("9.9.9-stale")); // drift; toolchain absent
    let before = f.read_lock();

    let out = f.run(&["build", "--locked", "--offline"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("lockfile is out of date"),
        "drift must win over the offline error under --frozen: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}

/// Under `--frozen`, when the lockfile is consistent and pinned (no drift) but
/// the artifact is absent, the offline error fires.
#[test]
fn frozen_reports_offline_when_consistent_but_absent() {
    let f = Fixture::new();
    f.manifest(&manifest_kotlin(KOTLIN))
        .lockfile(&lock_toolchain_pinned(KOTLIN)); // consistent + pinned, but absent
    let before = f.read_lock();

    let out = f.run(&["build", "--locked", "--offline"]);

    assert!(!out.success, "expected failure; stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("--offline") && !out.stderr.contains("lockfile is out of date"),
        "with no drift, --frozen should report the offline error: {}",
        out.stderr
    );
    assert_lock_unchanged(&f, &before);
}
