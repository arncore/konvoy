#!/usr/bin/env bash
# Konvoy smoke test suite.
#
# Each test_* function runs in a fresh temp directory.
# Tests call the `konvoy` binary directly and verify behavior.
set -uo pipefail

# ---------------------------------------------------------------------------
# Framework
# ---------------------------------------------------------------------------
PASS=0
FAIL=0
TOTAL=0
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

run_test() {
    local name="$1"
    TOTAL=$((TOTAL + 1))

    # Run each test in a subshell with a fresh temp directory.
    local tmpdir
    tmpdir=$(mktemp -d)

    if (cd "$tmpdir" && "$name") 2>&1; then
        PASS=$((PASS + 1))
        echo -e "  ${GREEN}PASS${NC}  $name"
    else
        FAIL=$((FAIL + 1))
        echo -e "  ${RED}FAIL${NC}  $name"
    fi

    rm -rf "$tmpdir"
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    if ! echo "$haystack" | grep -qF "$needle"; then
        echo "    expected output to contain: $needle" >&2
        echo "    got: $haystack" >&2
        return 1
    fi
}

assert_file_exists() {
    local path="$1"
    if [ ! -f "$path" ]; then
        echo "    expected file to exist: $path" >&2
        return 1
    fi
}

assert_dir_exists() {
    local path="$1"
    if [ ! -d "$path" ]; then
        echo "    expected directory to exist: $path" >&2
        return 1
    fi
}

assert_dir_not_exists() {
    local path="$1"
    if [ -d "$path" ]; then
        echo "    expected directory to not exist: $path" >&2
        return 1
    fi
}

assert_file_contains() {
    local path="$1"
    local needle="$2"
    if ! grep -qF "$needle" "$path"; then
        echo "    expected $path to contain: $needle" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# Tests: CLI basics
# ---------------------------------------------------------------------------

test_help_shows_subcommands() {
    local output
    output=$(konvoy --help 2>&1)
    assert_contains "$output" "init"
    assert_contains "$output" "build"
    assert_contains "$output" "run"
    assert_contains "$output" "test"
    assert_contains "$output" "clean"
    assert_contains "$output" "doctor"
}

test_version() {
    local output
    output=$(konvoy --version 2>&1)
    assert_contains "$output" "konvoy"
}

test_init_help() {
    konvoy init --help >/dev/null 2>&1
}

test_build_help() {
    konvoy build --help >/dev/null 2>&1
}

test_doctor_help() {
    konvoy doctor --help >/dev/null 2>&1
}

# ---------------------------------------------------------------------------
# Tests: konvoy init
# ---------------------------------------------------------------------------

test_init_creates_project() {
    konvoy init --name hello >/dev/null 2>&1
    assert_file_exists hello/konvoy.toml
    assert_file_exists hello/src/main.kt
    assert_file_contains hello/konvoy.toml "hello"
    assert_file_contains hello/src/main.kt "fun main()"
}

test_init_manifest_is_valid() {
    konvoy init --name valid-proj >/dev/null 2>&1
    assert_file_contains valid-proj/konvoy.toml 'name = "valid-proj"'
    assert_file_contains valid-proj/konvoy.toml "src/main.kt"
}

test_init_double_fails() {
    konvoy init --name dup >/dev/null 2>&1
    local output
    if output=$(konvoy init --name dup 2>&1); then
        echo "    expected second init to fail" >&2
        return 1
    fi
    assert_contains "$output" "already exists"
}

# ---------------------------------------------------------------------------
# Tests: konvoy doctor
# ---------------------------------------------------------------------------

test_doctor_all_ok() {
    local output
    output=$(konvoy doctor 2>&1)
    assert_contains "$output" "[ok] Host target:"
    assert_contains "$output" "[ok] konanc:"
    assert_contains "$output" "All checks passed"
}

# ---------------------------------------------------------------------------
# Tests: konvoy build (full pipeline)
# ---------------------------------------------------------------------------

test_build_produces_binary() {
    konvoy init --name smoke-build >/dev/null 2>&1
    cd smoke-build
    konvoy build 2>&1
    assert_file_exists .konvoy/build/linux_x64/debug/smoke-build
}

test_build_cache_hit() {
    konvoy init --name cache-hit >/dev/null 2>&1
    cd cache-hit

    local out1
    out1=$(konvoy build 2>&1)
    assert_contains "$out1" "Compiling"

    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "Fresh"
}

test_build_release() {
    konvoy init --name rel >/dev/null 2>&1
    cd rel
    konvoy build --release 2>&1
    assert_file_exists .konvoy/build/linux_x64/release/rel
}

# ---------------------------------------------------------------------------
# Tests: konvoy run
# ---------------------------------------------------------------------------

test_run_executes() {
    konvoy init --name runner >/dev/null 2>&1
    cd runner
    local output
    output=$(konvoy run 2>/dev/null)
    assert_contains "$output" "Hello, runner!"
}

# ---------------------------------------------------------------------------
# Tests: konvoy clean
# ---------------------------------------------------------------------------

test_clean_removes_artifacts() {
    konvoy init --name cleaner >/dev/null 2>&1
    cd cleaner
    konvoy build >/dev/null 2>&1
    assert_dir_exists .konvoy
    konvoy clean >/dev/null 2>&1
    assert_dir_not_exists .konvoy
}

test_clean_then_rebuild() {
    konvoy init --name rebuild >/dev/null 2>&1
    cd rebuild
    konvoy build >/dev/null 2>&1
    konvoy clean >/dev/null 2>&1

    local output
    output=$(konvoy build 2>&1)
    assert_contains "$output" "Compiling"
}

test_clean_no_konvoy_dir_ok() {
    mkdir -p src
    printf '[package]\nname = "noop"\n' > konvoy.toml
    konvoy clean >/dev/null 2>&1
}

# ---------------------------------------------------------------------------
# Tests: lockfile
# ---------------------------------------------------------------------------

test_lockfile_created_after_build() {
    konvoy init --name locker >/dev/null 2>&1
    cd locker
    [ ! -f konvoy.lock ] || { echo "    lockfile should not exist before build" >&2; return 1; }
    konvoy build >/dev/null 2>&1
    assert_file_exists konvoy.lock
    assert_file_contains konvoy.lock "konanc_version"
}

# ---------------------------------------------------------------------------
# Tests: error cases
# ---------------------------------------------------------------------------

test_build_outside_project_fails() {
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail outside project" >&2
        return 1
    fi
    assert_contains "$output" "no konvoy.toml"
}

test_run_outside_project_fails() {
    local output
    if output=$(konvoy run 2>&1); then
        echo "    expected run to fail outside project" >&2
        return 1
    fi
    assert_contains "$output" "no konvoy.toml"
}

test_build_no_sources_fails() {
    mkdir -p src
    printf '[package]\nname = "empty"\n' > konvoy.toml
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with no sources" >&2
        return 1
    fi
    assert_contains "$output" "source"
}

# ---------------------------------------------------------------------------
# Run all tests
# ---------------------------------------------------------------------------
echo "Running Konvoy smoke tests..."
echo ""

# CLI basics
run_test test_help_shows_subcommands
run_test test_version
run_test test_init_help
run_test test_build_help
run_test test_doctor_help

# init
run_test test_init_creates_project
run_test test_init_manifest_is_valid
run_test test_init_double_fails

# doctor
run_test test_doctor_all_ok

# build pipeline
run_test test_build_produces_binary
run_test test_build_cache_hit
run_test test_build_release

# run
run_test test_run_executes

# clean
run_test test_clean_removes_artifacts
run_test test_clean_then_rebuild
run_test test_clean_no_konvoy_dir_ok

# lockfile
run_test test_lockfile_created_after_build

# error cases
run_test test_build_outside_project_fails
run_test test_run_outside_project_fails
run_test test_build_no_sources_fails

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "---"
echo -e "Results: ${GREEN}${PASS} passed${NC}, ${RED}${FAIL} failed${NC}, ${TOTAL} total"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
