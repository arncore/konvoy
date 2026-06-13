#!/usr/bin/env bash
# Konvoy smoke test suite.
#
# Each test_* function runs in a fresh temp directory.
# Tests call the `konvoy` binary directly and verify behavior.
# Tests run in parallel (up to $MAX_PARALLEL at once) for speed.
set -uo pipefail

# ---------------------------------------------------------------------------
# Framework
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

# Max parallel test jobs. Default to number of CPUs, capped at 8.
MAX_PARALLEL="${MAX_PARALLEL:-$(nproc 2>/dev/null || echo 4)}"
if [ "$MAX_PARALLEL" -gt 8 ]; then MAX_PARALLEL=8; fi

# Results directory for thread-safe result collection.
RESULTS_DIR=$(mktemp -d)
PIDS=()
TEST_NAMES=()

# Run a single test in the background.
run_test() {
    local name="$1"

    # Optional selective run: SMOKE_FILTER='<regex>' bash tests.sh
    if [ -n "${SMOKE_FILTER:-}" ] && ! echo "$name" | grep -qE "${SMOKE_FILTER}"; then
        return 0
    fi

    local result_file="$RESULTS_DIR/$name"

    (
        local tmpdir
        tmpdir=$(mktemp -d)
        local log_file="$RESULTS_DIR/${name}.log"
        # Run the test body under `set -e` in a standalone subshell. The old
        # form — the subshell as an `if` condition — made bash ignore errexit
        # inside the entire test, so only each test's LAST command decided
        # pass/fail and every mid-test assert_* failure was silently ignored.
        (set -e; cd "$tmpdir"; "$name") >"$log_file" 2>&1
        if [ $? -eq 0 ]; then
            echo "pass" > "$result_file"
        else
            echo "fail" > "$result_file"
        fi
        rm -rf "$tmpdir"
    ) &

    PIDS+=($!)
    TEST_NAMES+=("$name")

    # Throttle: if we've hit the parallelism cap, wait for one to finish.
    if [ "${#PIDS[@]}" -ge "$MAX_PARALLEL" ]; then
        wait_and_drain
    fi
}

# Wait for all running jobs and drain the queue, printing results.
wait_and_drain() {
    for i in "${!PIDS[@]}"; do
        wait "${PIDS[$i]}" 2>/dev/null || true
        local name="${TEST_NAMES[$i]}"
        local result_file="$RESULTS_DIR/$name"
        local log_file="$RESULTS_DIR/${name}.log"
        if [ -f "$result_file" ] && [ "$(cat "$result_file")" = "pass" ]; then
            echo -e "  ${GREEN}PASS${NC}  $name"
        else
            echo -e "  ${RED}FAIL${NC}  $name"
            # Show captured output for debugging.
            if [ -f "$log_file" ] && [ -s "$log_file" ]; then
                sed 's/^/         /' "$log_file"
            fi
        fi
    done
    PIDS=()
    TEST_NAMES=()
}

# Wait for all remaining jobs and print final summary.
finish_tests() {
    wait_and_drain

    local pass=0 fail=0 total=0
    for f in "$RESULTS_DIR"/*; do
        [ -f "$f" ] || continue
        # Skip log files — only count result files.
        case "$f" in *.log) continue ;; esac
        total=$((total + 1))
        if [ "$(cat "$f")" = "pass" ]; then
            pass=$((pass + 1))
        else
            fail=$((fail + 1))
        fi
    done
    rm -rf "$RESULTS_DIR"

    echo ""
    echo "---"
    echo -e "Results: ${GREEN}${pass} passed${NC}, ${RED}${fail} failed${NC}, ${total} total"

    if [ "$fail" -gt 0 ]; then
        exit 1
    fi
}

assert_contains() {
    local haystack="$1"
    local needle="$2"
    # `--` so needles starting with `-` (e.g. "--offline ...") are not
    # parsed as grep options.
    if ! echo "$haystack" | grep -qF -- "$needle"; then
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

assert_not_contains() {
    local haystack="$1"
    local needle="$2"
    if echo "$haystack" | grep -qF -- "$needle"; then
        echo "    expected output NOT to contain: $needle" >&2
        echo "    got: $haystack" >&2
        return 1
    fi
}

assert_file_contains() {
    local path="$1"
    local needle="$2"
    if ! grep -qF -- "$needle" "$path"; then
        echo "    expected $path to contain: $needle" >&2
        return 1
    fi
}

assert_file_not_contains() {
    local path="$1"
    local needle="$2"
    if grep -qF -- "$needle" "$path"; then
        echo "    expected $path NOT to contain: $needle" >&2
        return 1
    fi
}

assert_files_identical() {
    local a="$1"
    local b="$2"
    if ! cmp -s "$a" "$b"; then
        echo "    expected files to be byte-identical: $a vs $b" >&2
        diff "$a" "$b" 2>&1 | sed 's/^/      /' >&2 || true
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
    assert_contains "$output" "update"
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

    # Clean removes build artifacts but preserves .konvoy/.
    assert_dir_exists .konvoy
    assert_dir_exists .konvoy/build
    konvoy clean >/dev/null 2>&1
    assert_dir_not_exists .konvoy/build
    assert_dir_exists .konvoy

    # Rebuild after clean restores the binary (may be a cache hit from
    # the global content-addressed cache at ~/.konvoy/).
    konvoy build >/dev/null 2>&1
    assert_file_exists .konvoy/build/linux_x64/debug/lifecycle
}

test_build_release() {
    konvoy init --name rel >/dev/null 2>&1
    cd rel
    konvoy build --release 2>&1
    assert_file_exists .konvoy/build/linux_x64/release/rel
}

test_clean_no_konvoy_dir_ok() {
    mkdir -p src
    printf '[package]\nname = "noop"\n\n[toolchain]\nkotlin = "2.2.0"\n' > konvoy.toml
    konvoy clean >/dev/null 2>&1
}

test_clean_default_preserves_cache() {
    konvoy init --name clean-keep >/dev/null 2>&1
    cd clean-keep
    konvoy build >/dev/null 2>&1

    # Create a cache dir to simulate non-build state.
    mkdir -p .konvoy/cache
    echo '{}' > .konvoy/cache/key.json

    konvoy clean >/dev/null 2>&1

    assert_dir_not_exists .konvoy/build
    assert_dir_exists .konvoy/cache
    assert_file_exists .konvoy/cache/key.json
}

test_clean_all_removes_everything() {
    konvoy init --name clean-all >/dev/null 2>&1
    cd clean-all
    konvoy build >/dev/null 2>&1

    mkdir -p .konvoy/cache
    echo '{}' > .konvoy/cache/key.json

    konvoy clean --all >/dev/null 2>&1

    assert_dir_not_exists .konvoy
}

test_clean_all_no_konvoy_dir_ok() {
    mkdir -p src
    printf '[package]\nname = "noop"\n\n[toolchain]\nkotlin = "2.2.0"\n' > konvoy.toml
    konvoy clean --all >/dev/null 2>&1
}

test_clean_rebuild_after_default() {
    konvoy init --name clean-rebuild >/dev/null 2>&1
    cd clean-rebuild
    konvoy build >/dev/null 2>&1

    konvoy clean >/dev/null 2>&1
    assert_dir_not_exists .konvoy/build

    # Rebuild should succeed (may be a cache hit from global cache).
    konvoy build >/dev/null 2>&1
    assert_file_exists .konvoy/build/linux_x64/debug/clean-rebuild
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
    # A manifest with [plugins] using { maven, version } must be accepted.
    konvoy init --name plug-parse >/dev/null 2>&1
    cat >> plug-parse/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
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

test_plugin_empty_version_error() {
    konvoy init --name plug-ver >/dev/null 2>&1
    cat >> plug-ver/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin" }
TOML
    cd plug-ver
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with missing plugin version" >&2
        return 1
    fi
    assert_contains "$output" "version"
}

test_plugin_without_maven_fails() {
    # A plugin entry without `maven` should be rejected.
    konvoy init --name plug-no-maven >/dev/null 2>&1
    cat >> plug-no-maven/konvoy.toml << 'TOML'

[plugins]
bad = { version = "1.0.0" }
TOML
    cd plug-no-maven
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with missing maven coordinate" >&2
        return 1
    fi
    assert_contains "$output" "maven"
}

test_plugin_with_path_fails() {
    # Plugins must use maven coordinates, not path.
    konvoy init --name plug-path >/dev/null 2>&1
    cat >> plug-path/konvoy.toml << 'TOML'

[plugins]
bad = { path = "../foo" }
TOML
    cd plug-path
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with path-based plugin" >&2
        return 1
    fi
    assert_contains "$output" "path"
}

test_plugin_locked_no_entries_error() {
    # --locked must fail when plugins are declared but lockfile has no plugin entries.
    konvoy init --name plug-locked >/dev/null 2>&1
    cat >> plug-locked/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
TOML
    cd plug-locked
    # Write a valid lockfile that's missing plugin entries.
    printf '[toolchain]\nkonanc_version = "2.2.0"\n' > konvoy.lock
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

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }

[dependencies]
kotlinx-serialization-core = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-core", version = "1.8.0" }
kotlinx-serialization-json = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-json", version = "1.8.0" }
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

    # Resolve Maven dependencies first.
    konvoy update >/dev/null 2>&1

    # First build downloads plugin artifacts and compiles.
    local out1
    out1=$(konvoy build 2>&1)
    assert_contains "$out1" "Compiling"

    # Lockfile should contain plugin entries after build.
    assert_file_exists konvoy.lock
    assert_file_contains konvoy.lock "[[plugins]]"
    assert_file_contains konvoy.lock "kotlin-serialization"
    assert_file_contains konvoy.lock "sha256"
    # Plugin lockfile should have maven and version fields.
    assert_file_contains konvoy.lock "maven ="
    assert_file_contains konvoy.lock "version ="

    # A second unchanged build MUST be a cache hit, not a recompile (issue #133
    # class). The plugin lock entries are folded into the cache key on the first
    # build, so the first and second builds compute the same key.
    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "(cached)"
    assert_not_contains "$out2" "Compiling"

    # --locked should also be a cache hit now that the lockfile has plugin
    # entries (and must not error out).
    local out3
    out3=$(konvoy build --locked 2>&1)
    assert_contains "$out3" "(cached)"
    assert_not_contains "$out3" "Compiling"
}

test_path_dep_plugin_build_lifecycle() {
    # Path-dep [plugins] are applied to the dep's own compile and pinned in the
    # ROOT lockfile (#293; the root here declares no plugins of its own). Uses
    # the canonical serialization plugin coordinate, same as the rest of the
    # suite (the -embeddable variant was reverted in #245 as "not the real fix";
    # the real #239 fix was two-step program compilation, #243 — and library
    # path-deps compile single-step with -Xplugin regardless).
    konvoy init --name plug-dep-lib --lib >/dev/null 2>&1
    konvoy init --name plug-dep-app >/dev/null 2>&1
    cat >> plug-dep-lib/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
TOML
    cat >> plug-dep-app/konvoy.toml << 'TOML'

[dependencies]
plug-dep-lib = { path = "../plug-dep-lib" }
TOML

    # Application probe: a @Serializable class (matched by FQ name) gives the
    # serialization plugin something to act on. With the runtime library absent
    # the dep's compile then FAILS — a failure localized to the dep's own
    # compilation is positive, platform-independent proof the dep was built with
    # its -Xplugin. Before #293 the dep compiled with no plugins, so this build
    # SUCCEEDED (silently wrong). PART 2 below flips the result by removing only
    # the @Serializable usage (plugin still declared) and the build succeeds —
    # confirming the failure was the plugin engaging, not the annotation itself.
    cat > plug-dep-lib/src/lib.kt << 'KOTLIN'
package kotlinx.serialization

@Target(AnnotationTarget.CLASS)
annotation class Serializable

@Serializable
data class User(val name: String, val age: Int)
KOTLIN

    cd plug-dep-app
    local probe
    if probe=$(konvoy build 2>&1); then
        echo "    expected the dep compile to fail under the serialization plugin (was -Xplugin applied to the dep?)" >&2
        return 1
    fi
    # The failure must be in the DEP's compilation (the dep got the plugin), so
    # the dep's compile line printed but the root's never did.
    assert_contains "$probe" "Compiling plug-dep-lib"
    assert_contains "$probe" "compilation failed"
    assert_not_contains "$probe" "Compiling plug-dep-app"

    # Standalone parity: the dep must fail the same way built on its own.
    local standalone
    if standalone=$(cd ../plug-dep-lib && konvoy build 2>&1); then
        echo "    expected the standalone dep build to fail under the serialization plugin" >&2
        return 1
    fi
    assert_contains "$standalone" "compilation failed"
    rm -rf ../plug-dep-lib/.konvoy ../plug-dep-lib/konvoy.lock

    # PART 2 — control + pinning/caching lifecycle. Same plugin still declared,
    # but the source no longer uses @Serializable, so the plugin is a clean
    # no-op and the build succeeds. That this now compiles (vs. PART 1's failure)
    # is the control proving the plugin was genuinely engaging in PART 1.
    cat > ../plug-dep-lib/src/lib.kt << 'KOTLIN'
package models

fun greet(): String = "hello"
KOTLIN
    local out1
    out1=$(konvoy build 2>&1)
    assert_contains "$out1" "Compiling plug-dep-lib"
    assert_contains "$out1" "Compiling plug-dep-app"

    # The dep's plugin pin lands in the ROOT lockfile; the dep checkout is
    # never written to by the root build.
    assert_file_contains konvoy.lock "[[plugins]]"
    assert_file_contains konvoy.lock "kotlin-serialization-compiler-plugin"
    assert_file_contains konvoy.lock "sha256"
    if [ -f ../plug-dep-lib/konvoy.lock ]; then
        echo "    root build must not write a lockfile into the dep checkout" >&2
        return 1
    fi

    # Second build: fully cached — the dep plugin pins and the path-dep entries
    # are folded into the first build's cache key (issue #133 class).
    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "(cached)"
    assert_not_contains "$out2" "Compiling"

    # --locked succeeds with the dep's pin present...
    local out3
    out3=$(konvoy build --locked 2>&1)
    assert_contains "$out3" "(cached)"

    # ...and fails actionably when the dep's pin is missing.
    sed -i '/\[\[plugins\]\]/,$d' konvoy.lock
    local out4
    if out4=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail with the dep plugin pin missing" >&2
        return 1
    fi
    assert_contains "$out4" "lockfile"
}

test_path_dep_plugin_union_dedup() {
    # The root AND a path-dep declare the SAME plugin (same coordinate + version
    # via {kotlin}). The graph-wide union is content-addressed by
    # (name, maven, version), so the shared plugin is pinned exactly ONCE in the
    # root lock — not duplicated, not a conflict (#293).
    konvoy init --name dedup-lib --lib >/dev/null 2>&1
    konvoy init --name dedup-app >/dev/null 2>&1
    local ser='kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }'
    printf '\n[plugins]\n%s\n' "$ser" >> dedup-lib/konvoy.toml
    printf '\n[plugins]\n%s\n\n[dependencies]\ndedup-lib = { path = "../dedup-lib" }\n' "$ser" >> dedup-app/konvoy.toml
    # No @Serializable anywhere — the plugin is a clean no-op, so both compiles
    # succeed and the lockfile is written.
    printf 'package dedup\nfun helper(): Int = 1\n' > dedup-lib/src/lib.kt
    cd dedup-app

    local out
    out=$(konvoy build 2>&1)
    assert_contains "$out" "Compiling dedup-lib"
    assert_contains "$out" "Compiling dedup-app"

    # Exactly one [[plugins]] entry despite the plugin being declared twice.
    local count
    count=$(grep -c '^\[\[plugins\]\]' konvoy.lock)
    if [ "$count" != "1" ]; then
        echo "    expected exactly 1 deduped plugin pin, got $count" >&2
        cat konvoy.lock >&2
        return 1
    fi
    assert_file_contains konvoy.lock "kotlin-serialization-compiler-plugin"

    # Second build is fully cached (the deduped pin is folded into the key).
    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "(cached)"
    assert_not_contains "$out2" "Compiling"
}

test_graph_plugin_union_different_plugins() {
    # The root declares one plugin and a path-dep declares a DIFFERENT one. The
    # root lock records the deduped UNION of both — each pinned once, sorted by
    # name — and no dependency checkout is written to (#293).
    konvoy init --name union-lib --lib >/dev/null 2>&1
    konvoy init --name union-app >/dev/null 2>&1
    # Dep uses allopen; root uses serialization. Both are no-ops on plain source
    # (no annotations / no @Serializable), so both projects compile cleanly.
    cat >> union-lib/konvoy.toml << 'TOML'

[plugins]
allopen = { maven = "org.jetbrains.kotlin:kotlin-allopen-compiler-plugin", version = "{kotlin}" }
TOML
    cat >> union-app/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }

[dependencies]
union-lib = { path = "../union-lib" }
TOML
    printf 'package unionlib\nfun helper(): Int = 1\n' > union-lib/src/lib.kt
    cd union-app

    local out
    out=$(konvoy build 2>&1)
    assert_contains "$out" "Compiling union-lib"
    assert_contains "$out" "Compiling union-app"

    # Both plugins pinned in the ROOT lock — two distinct entries.
    local count
    count=$(grep -c '^\[\[plugins\]\]' konvoy.lock)
    if [ "$count" != "2" ]; then
        echo "    expected 2 plugin pins in the union, got $count" >&2
        cat konvoy.lock >&2
        return 1
    fi
    assert_file_contains konvoy.lock "kotlin-allopen-compiler-plugin"
    assert_file_contains konvoy.lock "kotlin-serialization-compiler-plugin"

    # The dep contributed a pin to the root lock but its own checkout is never
    # written to.
    if [ -f ../union-lib/konvoy.lock ]; then
        echo "    root build must not write a lockfile into the dep checkout" >&2
        return 1
    fi

    # Cache-key stability with a multi-plugin union: second build is cached.
    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "(cached)"
    assert_not_contains "$out2" "Compiling"
}

test_transitive_dep_plugin_applied_and_pinned() {
    # A grandchild (depth-2) path-dep declares a plugin: app -> child -> gc.
    # The graph-wide union must reach the grandchild, apply its plugin to the
    # grandchild's OWN compile, and pin it in the ROOT (app) lockfile (#293).
    konvoy init --name gc-lib --lib >/dev/null 2>&1
    konvoy init --name child-lib --lib >/dev/null 2>&1
    konvoy init --name app-root >/dev/null 2>&1
    cat >> gc-lib/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
TOML
    cat >> child-lib/konvoy.toml << 'TOML'

[dependencies]
gc-lib = { path = "../gc-lib" }
TOML
    cat >> app-root/konvoy.toml << 'TOML'

[dependencies]
child-lib = { path = "../child-lib" }
TOML

    # Probe: a @Serializable class in the GRANDCHILD gives its plugin something
    # to act on; with the runtime absent the grandchild's compile then FAILS. A
    # failure localized to the grandchild's own compilation is positive proof the
    # plugin reached depth 2 of the graph. Before #293 the grandchild compiled
    # with no -Xplugin, so the build SUCCEEDED (silently wrong). The no-op rebuild
    # below (plugin still declared) flips it to success — the control.
    cat > gc-lib/src/lib.kt << 'KOTLIN'
package kotlinx.serialization

@Target(AnnotationTarget.CLASS)
annotation class Serializable

@Serializable
data class User(val name: String, val age: Int)
KOTLIN

    cd app-root
    local probe
    if probe=$(konvoy build 2>&1); then
        echo "    expected the grandchild compile to fail under the serialization plugin (did the union reach depth 2?)" >&2
        return 1
    fi
    # Failure is in the grandchild's compile (it got the plugin); the build never
    # reached the child or the root.
    assert_contains "$probe" "Compiling gc-lib"
    assert_contains "$probe" "compilation failed"
    assert_not_contains "$probe" "Compiling app-root"

    # Make the grandchild's plugin a no-op and verify the full pin lifecycle.
    cat > ../gc-lib/src/lib.kt << 'KOTLIN'
package gclib

fun greet(): String = "hello from grandchild"
KOTLIN
    local out1
    out1=$(konvoy build 2>&1)
    assert_contains "$out1" "Compiling gc-lib"
    assert_contains "$out1" "Compiling child-lib"
    assert_contains "$out1" "Compiling app-root"

    # The grandchild's plugin pin lands in the ROOT (app) lockfile; neither
    # intermediate checkout is written to.
    assert_file_contains konvoy.lock "[[plugins]]"
    assert_file_contains konvoy.lock "kotlin-serialization-compiler-plugin"
    if [ -f ../gc-lib/konvoy.lock ] || [ -f ../child-lib/konvoy.lock ]; then
        echo "    root build must not write lockfiles into dep checkouts" >&2
        return 1
    fi

    # Second build fully cached across the whole 3-level graph.
    local out2
    out2=$(konvoy build 2>&1)
    assert_contains "$out2" "(cached)"
    assert_not_contains "$out2" "Compiling"

    # --locked fails actionably if the grandchild's pin is removed from the lock.
    sed -i '/\[\[plugins\]\]/,$d' konvoy.lock
    local out3
    if out3=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail with the grandchild plugin pin missing" >&2
        return 1
    fi
    assert_contains "$out3" "lockfile"
}

test_plugin_kotlin_placeholder_resolves() {
    # version = "{kotlin}" should resolve to the toolchain version in the lockfile.
    konvoy init --name plug-placeholder >/dev/null 2>&1
    cat >> plug-placeholder/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
TOML
    cd plug-placeholder

    # Build to generate lockfile with resolved plugin entries.
    konvoy build >/dev/null 2>&1 || true

    # The lockfile must not contain the literal placeholder.
    if [ -f konvoy.lock ]; then
        assert_file_not_contains konvoy.lock "{kotlin}"
    fi
}

test_issue_239_serialization_plugin_applied() {
    # Issue #239: serialization compiler plugin is downloaded but not applied.
    # Exact reproducer from the bug report. The binary must run without
    # throwing a SerializationException.
    mkdir -p repro/src
    cat > repro/konvoy.toml << 'TOML'
[package]
name = "konvoy-serialization-repro"
kind = "bin"
entrypoint = "src/main.kt"

[toolchain]
kotlin = "2.2.0"

[plugins]
serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }

[dependencies]
kotlinx-serialization-core = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-core", version = "1.7.3" }
kotlinx-serialization-json = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-json", version = "1.7.3" }
TOML
    cat > repro/src/main.kt << 'KOTLIN'
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

@Serializable
data class Person(val name: String, val age: Int)

fun main() {
    val json = Json.decodeFromString<Person>("""{"name":"Alice","age":30}""")
    println("name=${json.name} age=${json.age}")
}
KOTLIN
    cd repro

    konvoy update >/dev/null 2>&1
    konvoy build 2>&1

    # The binary must run cleanly — no SerializationException.
    local run_output
    run_output=$(.konvoy/build/linux_x64/debug/konvoy-serialization-repro 2>&1)
    assert_contains "$run_output" "name=Alice age=30"
}

# ---------------------------------------------------------------------------
# Tests: Maven dependencies
# ---------------------------------------------------------------------------

test_update_help() {
    konvoy update --help >/dev/null 2>&1
}

test_update_no_maven_deps_noop() {
    # Running update on a project with only path deps is a no-op.
    konvoy init --name update-noop >/dev/null 2>&1
    cd update-noop
    local output
    output=$(konvoy update 2>&1)
    assert_contains "$output" "Updated 0 dependencies"
    # Lockfile should exist but have no Maven entries.
    assert_file_exists konvoy.lock
    assert_file_not_contains konvoy.lock "maven ="
}

test_update_resolves_maven_dep() {
    # konvoy update downloads klibs for all targets and writes hashes.
    konvoy init --name update-resolve >/dev/null 2>&1
    cat >> update-resolve/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd update-resolve
    local output
    output=$(konvoy update 2>&1)
    # ("Resolving <dep>" download bars are tty-only, so assert on the summary.)
    assert_contains "$output" "Updated 2 dependencies in konvoy.lock"

    # Lockfile should contain Maven dep entries.
    assert_file_exists konvoy.lock
    assert_file_contains konvoy.lock "kotlinx-datetime"
    assert_file_contains konvoy.lock "0.6.0"
    # Should have per-target hashes.
    assert_file_contains konvoy.lock "linux_x64"
    assert_file_contains konvoy.lock "linux_arm64"
    assert_file_contains konvoy.lock "macos_x64"
    assert_file_contains konvoy.lock "macos_arm64"
}

test_update_idempotent() {
    # Running update twice with the same version should skip re-download.
    konvoy init --name update-idem >/dev/null 2>&1
    cat >> update-idem/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd update-idem

    konvoy update >/dev/null 2>&1
    # Save lockfile content.
    local lock1
    lock1=$(cat konvoy.lock)

    # Second update should skip.
    local output
    output=$(konvoy update 2>&1)
    assert_contains "$output" "already up to date"

    # Lockfile should be identical.
    local lock2
    lock2=$(cat konvoy.lock)
    if [ "$lock1" != "$lock2" ]; then
        echo "    lockfile should not change on idempotent update" >&2
        return 1
    fi
}

test_update_multiple_deps() {
    # Update with multiple Maven deps at once.
    konvoy init --name update-multi >/dev/null 2>&1
    cat >> update-multi/konvoy.toml << 'TOML'

[dependencies]
kotlinx-coroutines = { maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd update-multi
    local output
    output=$(konvoy update 2>&1)
    # ("Resolving <dep>" download bars are tty-only, so assert on the summary:
    # 2 direct deps + their pinned transitives.)
    assert_contains "$output" "Updated 5 dependencies in konvoy.lock"

    assert_file_contains konvoy.lock "kotlinx-coroutines"
    assert_file_contains konvoy.lock "kotlinx-datetime"
}

test_update_preserves_path_deps() {
    # Path deps in the lockfile should survive a konvoy update.
    konvoy init --name path-lib --lib >/dev/null 2>&1
    konvoy init --name update-preserve >/dev/null 2>&1

    # First, build with only the path dep so it gets into the lockfile.
    printf '\n[dependencies]\npath-lib = { path = "../path-lib" }\n' >> update-preserve/konvoy.toml
    cd update-preserve
    konvoy build >/dev/null 2>&1
    assert_file_contains konvoy.lock "path-lib"
    assert_file_contains konvoy.lock 'source_type = "path"'

    # Now add a Maven dep and run update — path dep should be preserved.
    printf 'kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }\n' >> konvoy.toml
    konvoy update >/dev/null 2>&1
    assert_file_contains konvoy.lock "path-lib"
    assert_file_contains konvoy.lock 'source_type = "path"'
    assert_file_contains konvoy.lock "kotlinx-datetime"
}

test_update_version_change_re_resolves() {
    # Changing a dep version in the manifest should trigger re-download.
    konvoy init --name update-ver >/dev/null 2>&1
    cat >> update-ver/konvoy.toml << 'TOML'

[dependencies]
kotlinx-atomicfu = { maven = "org.jetbrains.kotlinx:atomicfu", version = "0.23.2" }
TOML
    cd update-ver

    konvoy update >/dev/null 2>&1
    local lock1
    lock1=$(cat konvoy.lock)

    # Change version.
    sed -i 's/0.23.2/0.26.1/' konvoy.toml
    konvoy update >/dev/null 2>&1
    local lock2
    lock2=$(cat konvoy.lock)

    # Lockfile should have changed (new hashes).
    if [ "$lock1" = "$lock2" ]; then
        echo "    lockfile should change when version changes" >&2
        return 1
    fi
    assert_file_contains konvoy.lock "0.26.1"
    assert_file_not_contains konvoy.lock "0.23.2"
}

test_update_removing_dep_cleans_lockfile() {
    # Removing a Maven dep from manifest and re-running update should
    # remove it from the lockfile.
    konvoy init --name update-remove >/dev/null 2>&1
    cat >> update-remove/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
kotlinx-atomicfu = { maven = "org.jetbrains.kotlinx:atomicfu", version = "0.26.1" }
TOML
    cd update-remove

    konvoy update >/dev/null 2>&1
    assert_file_contains konvoy.lock "kotlinx-datetime"
    assert_file_contains konvoy.lock "kotlinx-atomicfu"

    # Remove kotlinx-atomicfu from the manifest.
    sed -i '/kotlinx-atomicfu/d' konvoy.toml
    konvoy update >/dev/null 2>&1

    assert_file_contains konvoy.lock "kotlinx-datetime"
    assert_file_not_contains konvoy.lock "kotlinx-atomicfu"
}

test_maven_dep_manifest_both_path_and_maven_fails() {
    # A dependency with both path and maven should be rejected.
    konvoy init --name both-fail >/dev/null 2>&1
    cat >> both-fail/konvoy.toml << 'TOML'

[dependencies]
bad-dep = { path = "../bad", maven = "org.example:bad", version = "1.0.0" }
TOML
    cd both-fail
    local output
    if output=$(konvoy build 2>&1); then
        echo "    expected build to fail with ambiguous dep source" >&2
        return 1
    fi
    assert_contains "$output" "bad-dep"
}

test_build_maven_dep_without_update_succeeds() {
    # Building with a Maven dep without running `konvoy update` first should
    # auto-resolve the dependency and succeed.
    konvoy init --name no-update >/dev/null 2>&1
    cat >> no-update/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cat > no-update/src/main.kt << 'KOTLIN'
import kotlinx.datetime.Clock

fun main() {
    val now = Clock.System.now()
    println("Hello at $now")
}
KOTLIN
    cd no-update
    # Build without running update first — should auto-resolve.
    local output
    output=$(konvoy build 2>&1)
    assert_contains "$output" "Compiling"
    assert_file_exists .konvoy/build/linux_x64/debug/no-update
    # Lockfile should be created with the Maven dep entry.
    assert_file_exists konvoy.lock
    assert_file_contains konvoy.lock "kotlinx-datetime"
}

test_build_maven_dep_locked_without_update_fails() {
    # --locked must fail when Maven deps are declared but not in lockfile.
    konvoy init --name locked-maven >/dev/null 2>&1
    cat >> locked-maven/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd locked-maven
    # Create a lockfile with toolchain but no Maven entries.
    printf '[toolchain]\nkonanc_version = "2.2.0"\n' > konvoy.lock
    local output
    if output=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail without Maven entries in lockfile" >&2
        return 1
    fi
    assert_contains "$output" "lockfile"
}

test_maven_dep_build_lifecycle() {
    # End-to-end: update → build → cache hit → lockfile preserved.
    konvoy init --name maven-app >/dev/null 2>&1
    cat >> maven-app/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    # Source that uses kotlinx-datetime (import proves the klib was linked).
    cat > maven-app/src/main.kt << 'KOTLIN'
import kotlinx.datetime.Clock

fun main() {
    val now = Clock.System.now()
    println("Hello at $now")
}
KOTLIN
    cd maven-app

    # Step 1: update resolves the dep ("Resolving" bars are tty-only).
    local update_out
    update_out=$(konvoy update 2>&1)
    assert_contains "$update_out" "dependencies in konvoy.lock"
    assert_file_exists konvoy.lock

    # Step 2: build downloads only the host target klib and compiles.
    local build_out
    build_out=$(konvoy build 2>&1)
    assert_contains "$build_out" "Compiling"
    assert_file_exists .konvoy/build/linux_x64/debug/maven-app

    # Step 3: second build is a cache hit (no re-download).
    local build2_out
    build2_out=$(konvoy build 2>&1)
    assert_contains "$build2_out" "Fresh"

    # Step 4: run produces output (proves the binary works).
    local run_out
    run_out=$(konvoy run 2>/dev/null)
    assert_contains "$run_out" "Hello at"

    # Step 5: --locked build succeeds (lockfile has all entries).
    konvoy build --locked 2>&1

    # Step 6: lockfile still has Maven entries after build.
    assert_file_contains konvoy.lock "kotlinx-datetime"
    assert_file_contains konvoy.lock "maven ="
}

test_maven_dep_mixed_with_path_dep_build() {
    # A project with both path deps and Maven deps should build correctly.
    konvoy init --name mix-lib --lib >/dev/null 2>&1
    konvoy init --name mix-app >/dev/null 2>&1
    cat >> mix-app/konvoy.toml << 'TOML'

[dependencies]
mix-lib = { path = "../mix-lib" }
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cat > mix-app/src/main.kt << 'KOTLIN'
import kotlinx.datetime.Clock

fun main() {
    val now = Clock.System.now()
    println("Mixed at $now")
}
KOTLIN
    cd mix-app

    konvoy update >/dev/null 2>&1
    local output
    output=$(konvoy build 2>&1)
    assert_contains "$output" "Compiling mix-lib"
    assert_contains "$output" "Compiling mix-app"
    assert_file_exists .konvoy/build/linux_x64/debug/mix-app

    # Lockfile should have both path and Maven entries. Sibling path deps are
    # stored RELATIVE (`../mix-lib`), never as machine-specific absolute paths
    # — a committed lockfile must work at any checkout location under --locked.
    assert_file_contains konvoy.lock "mix-lib"
    assert_file_contains konvoy.lock 'source_type = "path"'
    assert_file_contains konvoy.lock 'path = "../mix-lib"'
    assert_file_contains konvoy.lock "kotlinx-datetime"
    assert_file_contains konvoy.lock "maven ="
}

test_doctor_maven_dep_checks() {
    # Doctor should check Maven deps and lockfile entries.
    konvoy init --name doc-maven >/dev/null 2>&1
    cat >> doc-maven/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd doc-maven
    konvoy update >/dev/null 2>&1

    local output
    output=$(konvoy doctor 2>&1)
    assert_contains "$output" "[ok] Maven dep: kotlinx-datetime"
    assert_contains "$output" "[ok] Lockfile entry: kotlinx-datetime"
}

test_doctor_missing_lockfile_entry_warns() {
    # Doctor should warn when Maven dep exists but lockfile has no entry.
    konvoy init --name doc-missing >/dev/null 2>&1
    cat >> doc-missing/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd doc-missing
    # Create lockfile without Maven entries.
    printf '[toolchain]\nkonanc_version = "2.2.0"\n' > konvoy.lock

    # Doctor exits non-zero when it reports problems — that's the point here.
    local output
    output=$(konvoy doctor 2>&1) || true
    assert_contains "$output" "not found"
    assert_contains "$output" "konvoy update"
}

test_doctor_no_lockfile_warns() {
    # Doctor should warn when Maven deps exist but no lockfile at all.
    konvoy init --name doc-nolock >/dev/null 2>&1
    cat >> doc-nolock/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd doc-nolock

    # Doctor exits non-zero when it reports problems — that's the point here.
    local output
    output=$(konvoy doctor 2>&1) || true
    assert_contains "$output" "No konvoy.lock"
    assert_contains "$output" "konvoy update"
}

# ---------------------------------------------------------------------------
# Tests: konvoy test
# ---------------------------------------------------------------------------

test_test_lifecycle() {
    # Build and run tests using konvoy test.
    konvoy init --name test-proj >/dev/null 2>&1
    cd test-proj

    # Create test source files.
    mkdir -p src/test
    cat > src/test/SampleTest.kt << 'KOTLIN'
import kotlin.test.Test
import kotlin.test.assertEquals

class SampleTest {
    @Test
    fun addition_works() {
        assertEquals(4, 2 + 2)
    }

    @Test
    fun string_concat() {
        assertEquals("hello world", "hello" + " " + "world")
    }
}
KOTLIN

    # First test build compiles.
    local out1
    out1=$(konvoy test 2>&1)
    assert_contains "$out1" "Compiling"

    # Second test build should be a cache hit.
    local out2
    out2=$(konvoy test 2>&1)
    assert_contains "$out2" "Fresh"
}

test_test_no_test_dir_fails() {
    konvoy init --name no-tests >/dev/null 2>&1
    cd no-tests
    local output
    if output=$(konvoy test 2>&1); then
        echo "    expected test to fail without src/test/ directory" >&2
        return 1
    fi
    assert_contains "$output" "no test source files"
}

test_test_filter() {
    # --filter should be forwarded to the test binary.
    konvoy init --name filter-proj >/dev/null 2>&1
    cd filter-proj
    mkdir -p src/test
    cat > src/test/FilterTest.kt << 'KOTLIN'
import kotlin.test.Test
import kotlin.test.assertTrue

class FilterTest {
    @Test
    fun included_test() {
        assertTrue(true)
    }

    @Test
    fun excluded_test() {
        assertTrue(true)
    }
}
KOTLIN

    # Run with a filter — should succeed (the filter pattern is passed through).
    konvoy test --filter "included" 2>&1
}

# ---------------------------------------------------------------------------
# Tests: lint full execution
# ---------------------------------------------------------------------------

test_lint_detects_findings() {
    # Run detekt on code with a known issue (magic number).
    konvoy init --name lint-find >/dev/null 2>&1
    cat > lint-find/src/main.kt << 'KOTLIN'
fun main() {
    val x = 42
    if (x > 10) {
        println("magic number: $x")
    }
}
KOTLIN
    # Add detekt to toolchain.
    printf '\n' >> lint-find/konvoy.toml
    sed -i 's/\[toolchain\]/[toolchain]\n/' lint-find/konvoy.toml
    cat > lint-find/konvoy.toml << TOML
[package]
name = "lint-find"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    cd lint-find

    # Lint should fail with findings.
    local output
    if output=$(konvoy lint 2>&1); then
        # Some code may pass detekt — that's ok, just verify lint ran.
        assert_contains "$output" "No lint issues found"
    else
        # If it found issues, verify we get structured output.
        assert_contains "$output" "issue"
    fi
}

test_lint_verbose() {
    konvoy init --name lint-verb >/dev/null 2>&1
    cat > lint-verb/konvoy.toml << TOML
[package]
name = "lint-verb"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    cd lint-verb

    # --verbose should run without error (output may be empty if no findings).
    konvoy lint --verbose 2>&1 || true
}

test_lint_locked() {
    konvoy init --name lint-lock >/dev/null 2>&1
    cat > lint-lock/konvoy.toml << TOML
[package]
name = "lint-lock"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    cd lint-lock

    # First lint run to populate lockfile with detekt hash.
    konvoy lint 2>&1 || true
    assert_file_exists konvoy.lock
    assert_file_contains konvoy.lock "detekt_version"
    assert_file_contains konvoy.lock "detekt_jar_sha256"

    # --locked should succeed now that hash is in lockfile.
    konvoy lint --locked 2>&1 || true
}

test_lint_custom_config() {
    konvoy init --name lint-cfg >/dev/null 2>&1
    cat > lint-cfg/konvoy.toml << TOML
[package]
name = "lint-cfg"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    # Create a custom detekt config.
    cat > lint-cfg/my-detekt.yml << 'YAML'
build:
  maxIssues: 999
YAML
    cd lint-cfg

    # --config should accept the custom file.
    konvoy lint --config my-detekt.yml 2>&1 || true
}

test_lint_missing_config_fails() {
    konvoy init --name lint-nofile >/dev/null 2>&1
    cat > lint-nofile/konvoy.toml << TOML
[package]
name = "lint-nofile"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    cd lint-nofile
    local output
    if output=$(konvoy lint --config nonexistent.yml 2>&1); then
        echo "    expected lint to fail with missing config file" >&2
        return 1
    fi
    assert_contains "$output" "config file not found"
}

# ---------------------------------------------------------------------------
# Tests: build flags
# ---------------------------------------------------------------------------

test_build_force_bypasses_cache() {
    konvoy init --name force-proj >/dev/null 2>&1
    cd force-proj

    # First build populates cache.
    konvoy build >/dev/null 2>&1

    # Second build with --force should recompile, not use cache.
    local output
    output=$(konvoy build --force 2>&1)
    assert_contains "$output" "Compiling"
    assert_not_contains "$output" "Fresh"
}

test_build_verbose_shows_output() {
    konvoy init --name verbose-proj >/dev/null 2>&1
    cd verbose-proj

    # --verbose should succeed (may show compiler info lines).
    local output
    output=$(konvoy build --verbose 2>&1)
    assert_contains "$output" "Compiling"
}

test_locked_toolchain_mismatch_fails() {
    konvoy init --name locked-tc >/dev/null 2>&1
    cd locked-tc
    # Build to populate lockfile.
    konvoy build >/dev/null 2>&1
    assert_file_exists konvoy.lock

    # Manually change the konanc_version in lockfile to create a mismatch.
    sed -i 's/konanc_version = "2.2.0"/konanc_version = "9.9.9"/' konvoy.lock
    local output
    if output=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail with toolchain mismatch" >&2
        return 1
    fi
    assert_contains "$output" "lockfile"
}

# ---------------------------------------------------------------------------
# Tests: init in-place
# ---------------------------------------------------------------------------

test_init_in_place() {
    # konvoy init without --name should initialize in the current directory.
    mkdir myproject
    cd myproject
    konvoy init >/dev/null 2>&1
    assert_file_exists konvoy.toml
    assert_file_exists src/main.kt
    assert_file_contains konvoy.toml 'name = "myproject"'
}

test_init_in_place_lib() {
    mkdir mylib
    cd mylib
    konvoy init --lib >/dev/null 2>&1
    assert_file_exists konvoy.toml
    assert_file_exists src/lib.kt
    assert_file_contains konvoy.toml 'kind = "lib"'
}

# ---------------------------------------------------------------------------
# Tests: transitive Maven dependencies
# ---------------------------------------------------------------------------

test_update_transitive_deps_in_lockfile() {
    # kotlinx-coroutines has transitive deps (atomicfu).
    # Verify they appear in the lockfile with required_by.
    konvoy init --name trans-deps >/dev/null 2>&1
    cat >> trans-deps/konvoy.toml << 'TOML'

[dependencies]
kotlinx-coroutines = { maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }
TOML
    cd trans-deps
    konvoy update >/dev/null 2>&1

    # Lockfile should contain the transitive dep (atomicfu).
    assert_file_contains konvoy.lock "atomicfu"
    # Transitive deps should have required_by populated.
    assert_file_contains konvoy.lock "required_by"
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
    printf '[package]\nname = "empty"\n\n[toolchain]\nkotlin = "2.2.0"\n' > konvoy.toml
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
    assert_file_exists .konvoy/build/linux_x64/debug/wide-app
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
    printf '[package]\nname = "empty"\n\n[toolchain]\nkotlin = "2.2.0"\ndetekt = "1.23.7"\n' > konvoy.toml
    printf '[toolchain]\nkonanc_version = "2.2.0"\n' > konvoy.lock
    local output
    if ! output=$(konvoy lint 2>&1); then
        echo "    expected lint to succeed with no sources" >&2
        return 1
    fi
    assert_contains "$output" "no Kotlin sources to lint"
}

# ---------------------------------------------------------------------------
# Tests: --locked / --offline reproducibility (issue #295)
# ---------------------------------------------------------------------------
# --locked  = reproducible install: never modify konvoy.lock; pinned artifacts
#             are still downloaded + SHA-verified; only lockfile drift errors.
# --offline = no network: every managed artifact must already be cached.
# They combine freely (--locked --offline == Cargo's --frozen).
#
# Tests that delete from the shared ~/.konvoy cache use dependency/detekt
# versions unique to that test, so parallel tests are unaffected.

test_repro_build_lifecycle() {
    # The everyday flow: build once online, then --offline / --locked /
    # --locked --offline all work as cache hits and never touch the lockfile.
    konvoy init --name repro-life >/dev/null 2>&1
    cd repro-life
    mkdir -p src/test
    cat > src/test/ReproTest.kt << 'KOTLIN'
import kotlin.test.Test
import kotlin.test.assertEquals

class ReproTest {
    @Test
    fun two_plus_two() {
        assertEquals(4, 2 + 2)
    }
}
KOTLIN

    local out1
    out1=$(konvoy build 2>&1) || { echo "$out1" >&2; return 1; }
    assert_contains "$out1" "Compiling"
    cp konvoy.lock lock.bak

    local out2
    out2=$(konvoy build --offline 2>&1) || { echo "$out2" >&2; return 1; }
    assert_contains "$out2" "Fresh"

    local out3
    out3=$(konvoy build --locked 2>&1) || { echo "$out3" >&2; return 1; }
    assert_contains "$out3" "Fresh"

    # Strictest mode (Cargo's --frozen): no lockfile changes AND no network.
    local out4
    out4=$(konvoy build --locked --offline 2>&1) || { echo "$out4" >&2; return 1; }
    assert_contains "$out4" "Fresh"
    assert_files_identical konvoy.lock lock.bak

    # A full recompile needs no network either: everything is cached.
    local out5
    out5=$(konvoy build --force --offline 2>&1) || { echo "$out5" >&2; return 1; }
    assert_contains "$out5" "Compiling"

    # run and test honor --offline through the same pipeline.
    local run_out
    run_out=$(konvoy run --offline 2>/dev/null) || { echo "$run_out" >&2; return 1; }
    assert_contains "$run_out" "Hello, repro-life!"

    konvoy test >/dev/null 2>&1
    local test_out
    test_out=$(konvoy test --offline 2>&1) || { echo "$test_out" >&2; return 1; }
    assert_contains "$test_out" "Fresh"

    assert_files_identical konvoy.lock lock.bak
}

test_offline_toolchain_absent_fails_fast() {
    # Clean machine + --offline: a toolchain that was never installed is a
    # hard, fast, actionable error — no download attempt, no lockfile created.
    mkdir -p src
    printf 'fun main() { println("hi") }\n' > src/main.kt
    printf '[package]\nname = "off-tc"\n\n[toolchain]\nkotlin = "2.1.0"\n' > konvoy.toml

    local output
    if output=$(konvoy build --offline 2>&1); then
        echo "    expected --offline build to fail with absent toolchain" >&2
        return 1
    fi
    assert_contains "$output" "Kotlin/Native toolchain 2.1.0 is not installed"
    assert_contains "$output" "--offline prevents downloads"
    assert_not_contains "$output" "Installing"
}

test_offline_unresolved_maven_dep_fails() {
    # --offline must refuse the automatic `konvoy update` for a Maven dep that
    # is not in the lockfile — resolving it would hit Maven Central.
    konvoy init --name off-dep >/dev/null 2>&1
    cat >> off-dep/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cd off-dep

    local output
    if output=$(konvoy build --offline 2>&1); then
        echo "    expected --offline build to fail with unresolved Maven dep" >&2
        return 1
    fi
    assert_contains "$output" 'dependency `kotlinx-datetime` is not resolved'
    assert_contains "$output" "konvoy update"
    # The refused auto-update must not have created a lockfile.
    if [ -f konvoy.lock ]; then
        echo "    --offline must not write konvoy.lock" >&2
        return 1
    fi
}

test_offline_build_after_update_succeeds() {
    # The provisioning flow: `konvoy update` online once, then the entire
    # build (klib resolution + compile + link) works offline.
    konvoy init --name off-maven >/dev/null 2>&1
    cat >> off-maven/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
TOML
    cat > off-maven/src/main.kt << 'KOTLIN'
import kotlinx.datetime.Clock

fun main() {
    println("Offline at ${Clock.System.now()}")
}
KOTLIN
    cd off-maven

    konvoy update >/dev/null 2>&1

    local output
    output=$(konvoy build --offline 2>&1) || { echo "$output" >&2; return 1; }
    assert_contains "$output" "Compiling"
    local binary
    binary=$(find .konvoy/build -name off-maven -type f | head -n 1)
    if [ -z "$binary" ]; then
        echo "    expected an off-maven binary under .konvoy/build" >&2
        return 1
    fi
}

test_offline_evicted_klib_fails() {
    # A pinned-but-evicted dependency klib under --offline is a hard error
    # naming the library (e.g. the cache was cleaned by hand or by CI).
    konvoy init --name off-evict >/dev/null 2>&1
    cat >> off-evict/konvoy.toml << 'TOML'

[dependencies]
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.1" }
TOML
    cd off-evict
    konvoy update >/dev/null 2>&1

    # Evict the cached klibs (version 0.6.1 is unique to this test).
    local cached
    cached=$(find "$HOME/.konvoy/cache" -name 'kotlinx-datetime-*0.6.1*' -type f | head -n 1)
    if [ -z "$cached" ]; then
        echo "    expected update to cache kotlinx-datetime 0.6.1 artifacts" >&2
        return 1
    fi
    find "$HOME/.konvoy/cache" -name 'kotlinx-datetime-*0.6.1*' -type f -delete

    local output
    if output=$(konvoy build --offline 2>&1); then
        echo "    expected --offline build to fail with evicted klib" >&2
        return 1
    fi
    assert_contains "$output" 'library `kotlinx-datetime` is not downloaded'
    assert_contains "$output" "--offline prevents downloads"
}

test_offline_lint_detekt_jar_absent_fails() {
    # detekt configured but its JAR never downloaded: lint --offline fails
    # fast, naming the version (1.23.5 is unique to this test — never cached).
    konvoy init --name off-lint >/dev/null 2>&1
    cat > off-lint/konvoy.toml << 'TOML'
[package]
name = "off-lint"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.5"
TOML
    cd off-lint

    local output
    if output=$(konvoy lint --offline 2>&1); then
        echo "    expected lint --offline to fail with absent detekt JAR" >&2
        return 1
    fi
    assert_contains "$output" "detekt 1.23.5 is not downloaded"
    assert_contains "$output" "--offline prevents downloads"
}

test_offline_lint_jre_failure_keeps_lockfile() {
    # detekt JAR cached + pinned, but the toolchain that provides its JRE is
    # not installed: lint --offline fails at the JRE — and must NOT leave a
    # rewritten konvoy.lock behind (regression: the detekt hash used to be
    # persisted before JRE resolution).
    konvoy init --name off-jre >/dev/null 2>&1
    cat > off-jre/konvoy.toml << 'TOML'
[package]
name = "off-jre"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    cd off-jre

    # Online lint caches the JAR and pins its hash (findings are fine).
    konvoy lint >/dev/null 2>&1 || true
    assert_file_contains konvoy.lock "detekt_jar_sha256"
    cp konvoy.lock lock.bak

    # Point the manifest at a toolchain that is not installed.
    sed -i 's/kotlin = "2.2.0"/kotlin = "2.1.0"/' konvoy.toml

    local output
    if output=$(konvoy lint --offline 2>&1); then
        echo "    expected lint --offline to fail at JRE resolution" >&2
        return 1
    fi
    assert_contains "$output" "detekt needs its JRE"
    assert_contains "$output" "--offline prevents downloads"
    assert_files_identical konvoy.lock lock.bak
}

test_locked_without_lockfile_fails() {
    # Forgot to commit konvoy.lock: --locked must fail up front, not resolve
    # silently — and must not create the lockfile as a side effect.
    konvoy init --name locked-nolock >/dev/null 2>&1
    cd locked-nolock

    local output
    if output=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail without a lockfile" >&2
        return 1
    fi
    assert_contains "$output" "lockfile is out of date"
    if [ -f konvoy.lock ]; then
        echo "    --locked must not create konvoy.lock" >&2
        return 1
    fi
}

test_locked_unpinned_toolchain_fails_before_install() {
    # Clean machine + a lockfile that pins only the toolchain VERSION (no
    # tarball SHAs, e.g. written by an older konvoy): --locked cannot install
    # reproducibly, so it reports drift fast — before attempting the ~500MB
    # toolchain download.
    mkdir -p src
    printf 'fun main() { println("hi") }\n' > src/main.kt
    printf '[package]\nname = "locked-unpinned"\n\n[toolchain]\nkotlin = "2.1.0"\n' > konvoy.toml
    printf '[toolchain]\nkonanc_version = "2.1.0"\n' > konvoy.lock
    cp konvoy.lock lock.bak

    local output
    if output=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail with an unpinned absent toolchain" >&2
        return 1
    fi
    assert_contains "$output" "lockfile is out of date"
    assert_not_contains "$output" "Installing"
    assert_files_identical konvoy.lock lock.bak
}

test_locked_redownloads_pinned_absent_klib() {
    # The #295 unification: --locked means reproducible INSTALL, not offline.
    # A pinned-but-evicted klib is re-downloaded (and SHA-verified) instead of
    # erroring, and konvoy.lock stays byte-identical.
    konvoy init --name locked-refetch >/dev/null 2>&1
    cat >> locked-refetch/konvoy.toml << 'TOML'

[dependencies]
kotlinx-atomicfu = { maven = "org.jetbrains.kotlinx:atomicfu", version = "0.25.0" }
TOML
    cd locked-refetch
    konvoy update >/dev/null 2>&1
    konvoy build >/dev/null 2>&1
    cp konvoy.lock lock.bak

    # Evict the cached klibs (version 0.25.0 is unique to this test).
    local cached
    cached=$(find "$HOME/.konvoy/cache" -name 'atomicfu-*0.25.0*' -type f | head -n 1)
    if [ -z "$cached" ]; then
        echo "    expected atomicfu 0.25.0 artifacts in the cache" >&2
        return 1
    fi
    find "$HOME/.konvoy/cache" -name 'atomicfu-*0.25.0*' -type f -delete

    # --locked re-downloads the pinned klib and succeeds.
    local output
    output=$(konvoy build --locked 2>&1) || { echo "$output" >&2; return 1; }
    assert_not_contains "$output" "lockfile is out of date"

    # The klib is back in the cache and the lockfile is untouched.
    cached=$(find "$HOME/.konvoy/cache" -name 'atomicfu-*0.25.0*.klib' -type f | head -n 1)
    if [ -z "$cached" ]; then
        echo "    expected --locked build to restore the evicted klib" >&2
        return 1
    fi
    assert_files_identical konvoy.lock lock.bak
}

test_locked_redownloads_pinned_absent_detekt_jar() {
    # Same unification for detekt: a pinned-but-deleted JAR is re-downloaded
    # and hash-verified under --locked (this used to be a hard error). Version
    # 1.23.6 is unique to this test.
    konvoy init --name locked-detekt >/dev/null 2>&1
    cat > locked-detekt/konvoy.toml << 'TOML'
[package]
name = "locked-detekt"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.6"
TOML
    cd locked-detekt

    konvoy lint >/dev/null 2>&1 || true
    assert_file_contains konvoy.lock "detekt_jar_sha256"
    cp konvoy.lock lock.bak

    local jar="$HOME/.konvoy/tools/detekt/1.23.6/detekt-cli-1.23.6-all.jar"
    assert_file_exists "$jar"
    rm -rf "$HOME/.konvoy/tools/detekt/1.23.6"

    # Findings may make lint exit non-zero; the asserts pin the behavior.
    local output
    output=$(konvoy lint --locked 2>&1) || true
    assert_not_contains "$output" "lockfile is out of date"
    assert_not_contains "$output" "is not downloaded"
    assert_file_exists "$jar"
    assert_files_identical konvoy.lock lock.bak
}

test_lint_locked_offline_lifecycle() {
    # After one online lint, every reproducibility mode works; removing the
    # pinned hash from the lockfile turns --locked into a drift error.
    konvoy init --name lint-repro >/dev/null 2>&1
    cat > lint-repro/konvoy.toml << 'TOML'
[package]
name = "lint-repro"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    cd lint-repro

    konvoy lint >/dev/null 2>&1 || true
    assert_file_contains konvoy.lock "detekt_version"
    assert_file_contains konvoy.lock "detekt_jar_sha256"

    local out_locked out_offline out_frozen
    out_locked=$(konvoy lint --locked 2>&1) || true
    assert_not_contains "$out_locked" "lockfile is out of date"

    out_offline=$(konvoy lint --offline 2>&1) || true
    assert_not_contains "$out_offline" "--offline prevents downloads"

    out_frozen=$(konvoy lint --locked --offline 2>&1) || true
    assert_not_contains "$out_frozen" "lockfile is out of date"
    assert_not_contains "$out_frozen" "--offline prevents downloads"

    # Drop the pinned hash: --locked now reports drift (the lockfile would
    # have to change to re-record it).
    sed -i '/detekt_jar_sha256/d' konvoy.lock
    cp konvoy.lock lock.bak
    local out_drift
    if out_drift=$(konvoy lint --locked 2>&1); then
        echo "    expected lint --locked to fail after removing the pinned hash" >&2
        return 1
    fi
    assert_contains "$out_drift" "lockfile is out of date"
    assert_files_identical konvoy.lock lock.bak
}

test_frozen_drift_wins_over_offline() {
    # --locked --offline with BOTH problems (stale lockfile AND an absent
    # artifact): drift is the actionable root cause and must win.
    konvoy init --name frozen-drift >/dev/null 2>&1
    cd frozen-drift
    konvoy build >/dev/null 2>&1

    sed -i 's/konanc_version = "2.2.0"/konanc_version = "9.9.9"/' konvoy.lock
    cp konvoy.lock lock.bak

    local output
    if output=$(konvoy build --locked --offline 2>&1); then
        echo "    expected --frozen build to fail on lockfile drift" >&2
        return 1
    fi
    assert_contains "$output" "lockfile is out of date"
    assert_not_contains "$output" "--offline prevents downloads"
    assert_files_identical konvoy.lock lock.bak
}

test_frozen_offline_error_when_klib_absent() {
    # --locked --offline with a CONSISTENT lockfile but an evicted klib: no
    # drift to report, so the offline error names the missing artifact —
    # even though the compiled binary itself is still cached.
    konvoy init --name frozen-absent >/dev/null 2>&1
    cat >> frozen-absent/konvoy.toml << 'TOML'

[dependencies]
kotlinx-atomicfu = { maven = "org.jetbrains.kotlinx:atomicfu", version = "0.26.0" }
TOML
    cd frozen-absent
    konvoy update >/dev/null 2>&1
    konvoy build >/dev/null 2>&1
    cp konvoy.lock lock.bak

    # Evict the cached klibs (version 0.26.0 is unique to this test).
    local cached
    cached=$(find "$HOME/.konvoy/cache" -name 'atomicfu-*0.26.0*' -type f | head -n 1)
    if [ -z "$cached" ]; then
        echo "    expected atomicfu 0.26.0 artifacts in the cache" >&2
        return 1
    fi
    find "$HOME/.konvoy/cache" -name 'atomicfu-*0.26.0*' -type f -delete

    local output
    if output=$(konvoy build --locked --offline 2>&1); then
        echo "    expected --frozen build to fail with evicted klib" >&2
        return 1
    fi
    # The eviction removes both atomicfu and its cinterop companion klib
    # (same filename prefix); klibs resolve in parallel, so either lockfile
    # entry may report first — assert the offline error, not the exact name.
    assert_contains "$output" "is not downloaded"
    assert_contains "$output" "--offline prevents downloads"
    assert_not_contains "$output" "lockfile is out of date"
    assert_files_identical konvoy.lock lock.bak
}

test_offline_plugin_absent_fails() {
    # The plugin gate fires after toolchain resolution but before any compile,
    # so --offline + an uncached plugin is a hard error naming the plugin.
    # A fake version guarantees the artifact is never in the shared cache.
    konvoy init --name off-plugin >/dev/null 2>&1
    cat >> off-plugin/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "9.9.9-fake" }
TOML
    cd off-plugin

    local output
    if output=$(konvoy build --offline 2>&1); then
        echo "    expected --offline build to fail with absent plugin" >&2
        return 1
    fi
    assert_contains "$output" 'plugin `kotlin-serialization` is not downloaded'
    assert_contains "$output" "--offline prevents downloads"
}

test_locked_plugin_pinned_absent_attempts_download() {
    # The #295 unification for the third artifact class: under --locked, a
    # pinned-but-absent plugin must PROCEED TO DOWNLOAD — not report drift,
    # not report offline. The fake version 404s at Maven Central, so the
    # expected outcome is a download error (proof the gate let it through).
    konvoy init --name locked-plugin >/dev/null 2>&1
    cat >> locked-plugin/konvoy.toml << 'TOML'

[plugins]
kotlin-serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "9.9.9-fake" }
TOML
    cd locked-plugin
    # Handcraft a consistent lockfile that pins the plugin (non-empty sha).
    cat > konvoy.lock << 'LOCK'
[toolchain]
konanc_version = "2.2.0"

[[plugins]]
name = "kotlin-serialization"
maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin"
version = "9.9.9-fake"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
url = "https://repo1.maven.org/maven2/org/jetbrains/kotlin/kotlin-serialization-compiler-plugin/9.9.9-fake/kotlin-serialization-compiler-plugin-9.9.9-fake.jar"
LOCK
    cp konvoy.lock lock.bak

    local output
    if output=$(konvoy build --locked 2>&1); then
        echo "    expected --locked build to fail at the 404 download" >&2
        return 1
    fi
    assert_contains "$output" "cannot download plugin"
    assert_not_contains "$output" "lockfile is out of date"
    assert_not_contains "$output" "--offline prevents downloads"
    assert_files_identical konvoy.lock lock.bak
}

test_locked_changed_path_dep_source_errors() {
    # Editing a path dependency's sources after the lockfile was written is
    # drift --locked must catch: the recorded source hash no longer matches.
    konvoy init --name hashlib --lib >/dev/null 2>&1
    konvoy init --name hashapp >/dev/null 2>&1
    printf '\n[dependencies]\nhashlib = { path = "../hashlib" }\n' >> hashapp/konvoy.toml
    cd hashapp
    konvoy build >/dev/null 2>&1
    assert_file_contains konvoy.lock "hashlib"
    cp konvoy.lock lock.bak

    # Change the dependency's source.
    printf '\nfun extra(): Int = 1\n' >> ../hashlib/src/lib.kt

    local output
    if output=$(konvoy build --locked 2>&1); then
        echo "    expected --locked to fail after dep source change" >&2
        return 1
    fi
    assert_contains "$output" 'dependency `hashlib` source hash mismatch'
    assert_contains "$output" "remove --locked"
    assert_files_identical konvoy.lock lock.bak
}

test_locked_relocated_checkout_succeeds() {
    # THE committed-lockfile story end-to-end: a project pair (app + sibling
    # ../lib dep) built at one location must build --locked at a DIFFERENT
    # location with the same lockfile — sibling paths are stored relative, so
    # the lockfile is machine/location independent.
    mkdir checkout-a
    cd checkout-a
    konvoy init --name relo-lib --lib >/dev/null 2>&1
    konvoy init --name relo-app >/dev/null 2>&1
    printf '\n[dependencies]\nrelo-lib = { path = "../relo-lib" }\n' >> relo-app/konvoy.toml
    (cd relo-app && konvoy build >/dev/null 2>&1)
    assert_file_contains relo-app/konvoy.lock 'path = "../relo-lib"'

    # Same layout, different location, committed lockfile carried over.
    cd ..
    mkdir checkout-b
    cp -R checkout-a/relo-lib checkout-b/relo-lib
    mkdir checkout-b/relo-app
    cp checkout-a/relo-app/konvoy.toml checkout-b/relo-app/
    cp checkout-a/relo-app/konvoy.lock checkout-b/relo-app/
    cp -R checkout-a/relo-app/src checkout-b/relo-app/src
    cd checkout-b/relo-app

    local output
    output=$(konvoy build --locked 2>&1) || { echo "$output" >&2; return 1; }
    assert_not_contains "$output" "lockfile is out of date"
    assert_files_identical konvoy.lock ../../checkout-a/relo-app/konvoy.lock
}

test_locked_detekt_hash_mismatch_fails() {
    # Integrity under --locked: a cached detekt JAR that does not match the
    # pinned hash is rejected (tampered/rotated artifact), not silently used.
    konvoy init --name lint-tamper >/dev/null 2>&1
    cat > lint-tamper/konvoy.toml << 'TOML'
[package]
name = "lint-tamper"

[toolchain]
kotlin = "2.2.0"
detekt = "1.23.7"
TOML
    cd lint-tamper

    konvoy lint >/dev/null 2>&1 || true
    assert_file_contains konvoy.lock "detekt_jar_sha256"

    # Tamper with the pinned hash (project-local lockfile only — the shared
    # JAR cache is untouched, so parallel tests are unaffected).
    sed -i 's/detekt_jar_sha256 = ".*"/detekt_jar_sha256 = "1111111111111111111111111111111111111111111111111111111111111111"/' konvoy.lock
    cp konvoy.lock lock.bak

    local output
    if output=$(konvoy lint --locked 2>&1); then
        echo "    expected lint --locked to fail on pinned-hash mismatch" >&2
        return 1
    fi
    assert_contains "$output" "jar hash mismatch"
    assert_files_identical konvoy.lock lock.bak
}

test_update_has_no_offline_flag() {
    # `konvoy update` is inherently online (it resolves from Maven Central);
    # --offline must be rejected by the CLI, not silently ignored.
    konvoy init --name upd-off >/dev/null 2>&1
    cd upd-off
    local output
    if output=$(konvoy update --offline 2>&1); then
        echo '    expected `konvoy update --offline` to be rejected' >&2
        return 1
    fi
    assert_contains "$output" "unexpected argument"
}

test_help_documents_repro_flags() {
    # Every fetching command documents both reproducibility flags.
    local cmd output
    for cmd in build run test lint; do
        output=$(konvoy "$cmd" --help 2>&1)
        assert_contains "$output" "--locked"
        assert_contains "$output" "--offline"
    done
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
run_test test_clean_default_preserves_cache
run_test test_clean_all_removes_everything
run_test test_clean_all_no_konvoy_dir_ok
run_test test_clean_rebuild_after_default

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
run_test test_plugin_empty_version_error
run_test test_plugin_without_maven_fails
run_test test_plugin_with_path_fails
run_test test_plugin_locked_no_entries_error
run_test test_plugin_build_lifecycle
run_test test_path_dep_plugin_build_lifecycle
run_test test_path_dep_plugin_union_dedup
run_test test_graph_plugin_union_different_plugins
run_test test_transitive_dep_plugin_applied_and_pinned
run_test test_plugin_kotlin_placeholder_resolves

# Issue #239: serialization plugin downloaded but not applied (fixed by two-step
# program compilation, #243 — the embeddable-JAR mapping was reverted in #245).
run_test test_issue_239_serialization_plugin_applied

# Maven dependencies
run_test test_update_help
run_test test_update_no_maven_deps_noop
run_test test_update_resolves_maven_dep
run_test test_update_idempotent
run_test test_update_multiple_deps
run_test test_update_preserves_path_deps
run_test test_update_version_change_re_resolves
run_test test_update_removing_dep_cleans_lockfile
run_test test_maven_dep_manifest_both_path_and_maven_fails
run_test test_build_maven_dep_without_update_succeeds
run_test test_build_maven_dep_locked_without_update_fails
run_test test_maven_dep_build_lifecycle
run_test test_maven_dep_mixed_with_path_dep_build
run_test test_doctor_maven_dep_checks
run_test test_doctor_missing_lockfile_entry_warns
run_test test_doctor_no_lockfile_warns
# test command
run_test test_test_lifecycle
run_test test_test_no_test_dir_fails
run_test test_test_filter

# lint full execution
run_test test_lint_detects_findings
run_test test_lint_verbose
run_test test_lint_locked
run_test test_lint_custom_config
run_test test_lint_missing_config_fails

# build flags
run_test test_build_force_bypasses_cache
run_test test_build_verbose_shows_output
run_test test_locked_toolchain_mismatch_fails

# --locked / --offline reproducibility (issue #295)
run_test test_repro_build_lifecycle
run_test test_offline_toolchain_absent_fails_fast
run_test test_offline_unresolved_maven_dep_fails
run_test test_offline_build_after_update_succeeds
run_test test_offline_evicted_klib_fails
run_test test_offline_lint_detekt_jar_absent_fails
run_test test_offline_lint_jre_failure_keeps_lockfile
run_test test_locked_without_lockfile_fails
run_test test_locked_unpinned_toolchain_fails_before_install
run_test test_locked_redownloads_pinned_absent_klib
run_test test_locked_redownloads_pinned_absent_detekt_jar
run_test test_lint_locked_offline_lifecycle
run_test test_frozen_drift_wins_over_offline
run_test test_frozen_offline_error_when_klib_absent
run_test test_offline_plugin_absent_fails
run_test test_locked_plugin_pinned_absent_attempts_download
run_test test_locked_changed_path_dep_source_errors
run_test test_locked_relocated_checkout_succeeds
run_test test_locked_detekt_hash_mismatch_fails
run_test test_update_has_no_offline_flag
run_test test_help_documents_repro_flags

# init in-place
run_test test_init_in_place
run_test test_init_in_place_lib

# transitive Maven deps
run_test test_update_transitive_deps_in_lockfile

# error cases
run_test test_build_outside_project_fails
run_test test_run_outside_project_fails
run_test test_build_no_sources_fails

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
finish_tests
