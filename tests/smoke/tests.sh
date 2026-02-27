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
# Tests: plugins
# ---------------------------------------------------------------------------

test_init_no_plugins_section() {
    konvoy init --name no-plug >/dev/null 2>&1
    # Default init should NOT include a [plugins] section.
    if grep -q '\[plugins' no-plug/konvoy.toml; then
        echo "    konvoy init should not add [plugins] section by default" >&2
        return 1
    fi
}

test_plugin_manifest_parses() {
    # A manifest with [plugins.serialization] must be accepted by konvoy.
    konvoy init --name plug-parse >/dev/null 2>&1
    cat >> plug-parse/konvoy.toml << 'TOML'

[plugins.serialization]
version = "1.7.3"
modules = ["json"]
TOML
    cd plug-parse
    # Build will fail at plugin download (no cached artifacts), but must NOT
    # fail at manifest parsing. Look for TOML/parse errors as a signal.
    local output
    output=$(konvoy build 2>&1) || true
    if echo "$output" | grep -qi "unknown field\|parse error\|invalid.*toml"; then
        echo "    manifest with plugins section should parse cleanly" >&2
        echo "    got: $output" >&2
        return 1
    fi
}

test_plugin_unknown_name_error() {
    konvoy init --name plug-unknown >/dev/null 2>&1
    cat >> plug-unknown/konvoy.toml << 'TOML'

[plugins.nonexistent]
version = "1.0.0"
TOML
    cd plug-unknown
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with unknown plugin name" >&2
        return 1
    fi
    assert_contains "$output" "unknown plugin"
    assert_contains "$output" "nonexistent"
    # Error should list available plugins so the user knows what to use.
    assert_contains "$output" "serialization"
}

test_plugin_unknown_module_error() {
    konvoy init --name plug-mod >/dev/null 2>&1
    cat >> plug-mod/konvoy.toml << 'TOML'

[plugins.serialization]
version = "1.7.3"
modules = ["nonexistent_module"]
TOML
    cd plug-mod
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with unknown module name" >&2
        return 1
    fi
    assert_contains "$output" "unknown module"
    assert_contains "$output" "nonexistent_module"
}

test_plugin_empty_version_error() {
    konvoy init --name plug-ver >/dev/null 2>&1
    cat >> plug-ver/konvoy.toml << 'TOML'

[plugins.serialization]
version = ""
TOML
    cd plug-ver
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with empty plugin version" >&2
        return 1
    fi
    assert_contains "$output" "version"
}

test_plugin_locked_no_entries_error() {
    # --locked must fail when plugins are declared but lockfile has no plugin entries.
    konvoy init --name plug-locked >/dev/null 2>&1
    cat >> plug-locked/konvoy.toml << 'TOML'

[plugins.serialization]
version = "1.7.3"
TOML
    cd plug-locked
    # Write a valid lockfile that's missing plugin entries.
    printf '[toolchain]\nkonanc_version = "2.1.0"\n' > konvoy.lock
    local output
    if output=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail without plugin entries in lockfile" >&2
        return 1
    fi
    assert_contains "$output" "lockfile"
}

test_plugin_build_lifecycle() {
    # End-to-end: build with serialization plugin, verify lockfile + --locked.
    konvoy init --name plug-app >/dev/null 2>&1
    cat >> plug-app/konvoy.toml << 'TOML'

[plugins.serialization]
version = "1.7.3"
TOML
    # Write Kotlin source that uses the @Serializable annotation.
    cat > plug-app/src/main.kt << 'KOTLIN'
import kotlinx.serialization.Serializable

@Serializable
data class User(val name: String, val age: Int)

fun main() {
    println("Hello with serialization!")
}
KOTLIN
    cd plug-app

    # First build downloads plugin artifacts and compiles.
    local out1
    out1=$(konvoy build 2>&1)
    assert_contains "$out1" "Compiling"

    # Lockfile should contain plugin entries after build.
    assert_file_exists konvoy.lock
    assert_file_contains konvoy.lock "[[plugins]]"
    assert_file_contains konvoy.lock "serialization"
    assert_file_contains konvoy.lock "sha256"

    # --locked should succeed now that the lockfile has plugin entries.
    # (May recompile due to lockfile content change — that's fine,
    #  the key assertion is that --locked doesn't error out.)
    konvoy build --locked 2>&1
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

test_dep_build_diamond() {
    # Diamond: shared <- [utils, models], app depends on both utils and models.
    # shared and logging are independent leaves → built in parallel (level 0).
    # utils and models both depend on shared → built in parallel (level 1).
    konvoy init --name shared --lib >/dev/null 2>&1
    konvoy init --name logging --lib >/dev/null 2>&1
    konvoy init --name utils --lib >/dev/null 2>&1
    printf '\n[dependencies]\nshared = { path = "../shared" }\n' >> utils/konvoy.toml
    konvoy init --name models --lib >/dev/null 2>&1
    printf '\n[dependencies]\nshared = { path = "../shared" }\n' >> models/konvoy.toml
    konvoy init --name app >/dev/null 2>&1
    printf '\n[dependencies]\nutils = { path = "../utils" }\nmodels = { path = "../models" }\nlogging = { path = "../logging" }\n' >> app/konvoy.toml

    cd app
    local output
    output=$(konvoy build 2>&1)
    # All four deps must be compiled.
    assert_contains "$output" "Compiling shared"
    assert_contains "$output" "Compiling logging"
    assert_contains "$output" "Compiling utils"
    assert_contains "$output" "Compiling models"
    assert_contains "$output" "Compiling app"
    assert_file_exists .konvoy/build/linux_x64/debug/app
}

test_dep_build_wide() {
    # Three independent libs — all at the same level, built in parallel.
    konvoy init --name lib-a --lib >/dev/null 2>&1
    konvoy init --name lib-b --lib >/dev/null 2>&1
    konvoy init --name lib-c --lib >/dev/null 2>&1
    konvoy init --name wide-app >/dev/null 2>&1
    printf '\n[dependencies]\nlib-a = { path = "../lib-a" }\nlib-b = { path = "../lib-b" }\nlib-c = { path = "../lib-c" }\n' >> wide-app/konvoy.toml

    cd wide-app
    local output
    output=$(konvoy build 2>&1)
    assert_contains "$output" "Compiling lib-a"
    assert_contains "$output" "Compiling lib-b"
    assert_contains "$output" "Compiling lib-c"
    assert_contains "$output" "Compiling wide-app"

    # Second build should be fully cached.
    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "Fresh lib-a"
    assert_contains "$out2" "Fresh lib-b"
    assert_contains "$out2" "Fresh lib-c"
    assert_contains "$out2" "Fresh wide-app"
}

test_dep_build_chain() {
    # Linear chain: leaf -> mid -> app (strictly sequential levels).
    konvoy init --name chain-leaf --lib >/dev/null 2>&1
    konvoy init --name chain-mid --lib >/dev/null 2>&1
    printf '\n[dependencies]\nchain-leaf = { path = "../chain-leaf" }\n' >> chain-mid/konvoy.toml
    konvoy init --name chain-app >/dev/null 2>&1
    printf '\n[dependencies]\nchain-mid = { path = "../chain-mid" }\n' >> chain-app/konvoy.toml

    cd chain-app
    local output
    output=$(konvoy build 2>&1)
    assert_contains "$output" "Compiling chain-leaf"
    assert_contains "$output" "Compiling chain-mid"
    assert_contains "$output" "Compiling chain-app"
    assert_file_exists .konvoy/build/linux_x64/debug/chain-app
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
run_test test_dep_build_diamond
run_test test_dep_build_wide
run_test test_dep_build_chain
run_test test_run_lib_fails

# lint
run_test test_lint_help
run_test test_lint_without_config_fails
run_test test_lint_no_sources_warns

# plugins
run_test test_init_no_plugins_section
run_test test_plugin_manifest_parses
run_test test_plugin_unknown_name_error
run_test test_plugin_unknown_module_error
run_test test_plugin_empty_version_error
run_test test_plugin_locked_no_entries_error
run_test test_plugin_build_lifecycle

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
