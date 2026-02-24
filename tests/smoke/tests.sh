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
    assert_file_exists hello/.gitignore
    assert_file_contains hello/konvoy.toml "hello"
    assert_file_contains hello/src/main.kt "fun main()"
    assert_file_contains hello/.gitignore ".konvoy/"
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
    # Run doctor inside a konvoy project so it can check the managed toolchain.
    konvoy init --name doctor-proj >/dev/null 2>&1
    cd doctor-proj
    # Pre-install toolchain so doctor can find it.
    konvoy build >/dev/null 2>&1

    local output
    output=$(konvoy doctor 2>&1)
    assert_contains "$output" "[ok] Host target:"
    assert_contains "$output" "[ok] konanc:"
    assert_contains "$output" "All checks passed"
}

# ---------------------------------------------------------------------------
# Tests: build lifecycle (combined to minimize konanc invocations)
# ---------------------------------------------------------------------------

# Single project exercises: build → cache hit → run → lockfile → clean → rebuild
test_build_lifecycle() {
    konvoy init --name lifecycle >/dev/null 2>&1
    cd lifecycle

    # No lockfile before first build.
    [ ! -f konvoy.lock ] || { echo "    lockfile should not exist before build" >&2; return 1; }

    # First build compiles.
    local out1
    out1=$(konvoy build 2>&1)
    assert_contains "$out1" "Compiling"
    assert_file_exists .konvoy/build/linux_x64/debug/lifecycle

    # Second build is a cache hit.
    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "Fresh"

    # Lockfile created after build.
    assert_file_exists konvoy.lock
    assert_file_contains konvoy.lock "konanc_version"

    # Run produces expected output.
    local run_output
    run_output=$(konvoy run 2>/dev/null)
    assert_contains "$run_output" "Hello, lifecycle!"

    # Clean removes artifacts.
    assert_dir_exists .konvoy
    konvoy clean >/dev/null 2>&1
    assert_dir_not_exists .konvoy

    # Rebuild after clean recompiles.
    local out3
    out3=$(konvoy build 2>&1)
    assert_contains "$out3" "Compiling"
}

test_build_release() {
    konvoy init --name rel >/dev/null 2>&1
    cd rel
    konvoy build --release 2>&1
    assert_file_exists .konvoy/build/linux_x64/release/rel
}

test_clean_no_konvoy_dir_ok() {
    mkdir -p src
    printf '[package]\nname = "noop"\n\n[toolchain]\nkotlin = "2.1.0"\n' > konvoy.toml
    konvoy clean >/dev/null 2>&1
}

# ---------------------------------------------------------------------------
# Tests: git worktree cache sharing
# ---------------------------------------------------------------------------

test_worktree_cache_shared() {
    # Set up a git repo with a konvoy project.
    konvoy init --name wt-proj >/dev/null 2>&1
    cd wt-proj
    git init -q
    git config user.email "test@test.com"
    git config user.name "Test"
    git add -A
    git commit -q -m "initial"

    # Build in the main worktree (populates cache).
    local out1
    out1=$(konvoy build 2>&1)
    assert_contains "$out1" "Compiling"

    # Create a worktree on a new branch.
    git worktree add -q ../wt-branch -b wt-branch

    # Build in the worktree — should be a cache hit.
    local out2
    out2=$(cd ../wt-branch && konvoy build 2>&1)
    assert_contains "$out2" "Fresh"

    # Clean up worktree.
    git worktree remove -f ../wt-branch
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
    printf '[package]\nname = "empty"\n\n[toolchain]\nkotlin = "2.1.0"\n' > konvoy.toml
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with no sources" >&2
        return 1
    fi
    assert_contains "$output" "source"
}

# ---------------------------------------------------------------------------
# Tests: toolchain management
# ---------------------------------------------------------------------------

test_toolchain_list_empty() {
    local output
    output=$(konvoy toolchain list 2>&1)
    # Should not error, may show "No toolchains" or list existing ones.
    # Since the build lifecycle test may have installed one, just check it runs.
    [ $? -eq 0 ] || return 1
}

test_toolchain_install_from_manifest() {
    konvoy init --name tc-proj >/dev/null 2>&1
    cd tc-proj
    konvoy toolchain install 2>&1
    # Verify konanc is available via doctor.
    local output
    output=$(konvoy doctor 2>&1)
    assert_contains "$output" "[ok] konanc:"
}

test_toolchain_help() {
    konvoy toolchain --help >/dev/null 2>&1
}

test_init_includes_toolchain() {
    konvoy init --name tc-init >/dev/null 2>&1
    assert_file_contains tc-init/konvoy.toml "[toolchain]"
    assert_file_contains tc-init/konvoy.toml "kotlin"
}

# ---------------------------------------------------------------------------
# Tests: library projects and dependencies
# ---------------------------------------------------------------------------

test_init_lib() {
    konvoy init --name my-lib --lib >/dev/null 2>&1
    assert_file_exists my-lib/konvoy.toml
    assert_file_exists my-lib/src/lib.kt
    assert_file_contains my-lib/konvoy.toml 'kind = "lib"'
    assert_file_contains my-lib/konvoy.toml 'version = "0.1.0"'
}

test_build_lib() {
    konvoy init --name build-lib --lib >/dev/null 2>&1
    cd build-lib
    local output
    output=$(konvoy build 2>&1)
    assert_contains "$output" "Compiling"
    assert_file_exists .konvoy/build/linux_x64/debug/build-lib.klib
}

test_dep_build() {
    # Create a library project.
    konvoy init --name utils --lib >/dev/null 2>&1
    # Create a binary project that depends on the library.
    konvoy init --name app >/dev/null 2>&1
    # Add dependency to app's manifest.
    printf '\n[dependencies]\nutils = { path = "../utils" }\n' >> app/konvoy.toml
    # Update app's source to use the library.
    cat > app/src/main.kt << 'KOTLIN'
fun main() {
    println(greet("world"))
}
KOTLIN
    cd app
    local output
    output=$(konvoy build 2>&1)
    assert_contains "$output" "Compiling utils"
    assert_contains "$output" "Compiling app"
}

test_run_lib_fails() {
    konvoy init --name run-lib --lib >/dev/null 2>&1
    cd run-lib
    local output
    if output=$(konvoy run 2>&1); then
        echo "    expected run to fail for library project" >&2
        return 1
    fi
    assert_contains "$output" "library"
}

# ---------------------------------------------------------------------------
# Tests: lint
# ---------------------------------------------------------------------------

test_lint_help() {
    konvoy lint --help >/dev/null 2>&1
}

test_lint_without_config_fails() {
    konvoy init --name no-lint >/dev/null 2>&1
    cd no-lint
    local output
    if output=$(konvoy lint 2>&1); then
        echo "    expected lint to fail without detekt in [toolchain]" >&2
        return 1
    fi
    assert_contains "$output" "detekt"
    assert_contains "$output" "[toolchain]"
}

test_lint_no_sources_warns() {
    printf '[package]\nname = "empty"\n\n[toolchain]\nkotlin = "2.1.0"\ndetekt = "1.23.7"\n' > konvoy.toml
    printf '[toolchain]\nkonanc_version = "2.1.0"\n' > konvoy.lock
    local output
    if ! output=$(konvoy lint 2>&1); then
        echo "    expected lint to succeed with no sources" >&2
        return 1
    fi
    assert_contains "$output" "no Kotlin sources to lint"
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
run_test test_toolchain_help

# init
run_test test_init_creates_project
run_test test_init_manifest_is_valid
run_test test_init_double_fails
run_test test_init_includes_toolchain

# build lifecycle (build, cache, run, lockfile, clean, rebuild)
run_test test_build_lifecycle
run_test test_build_release
run_test test_clean_no_konvoy_dir_ok

# doctor (runs after build so toolchain is installed)
run_test test_doctor_all_ok

# toolchain
run_test test_toolchain_list_empty
run_test test_toolchain_install_from_manifest

# worktree
run_test test_worktree_cache_shared

# library projects
run_test test_init_lib
run_test test_build_lib
run_test test_dep_build
run_test test_run_lib_fails

# lint
run_test test_lint_help
run_test test_lint_without_config_fails
run_test test_lint_no_sources_warns

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
