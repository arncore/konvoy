# Konvoy

[![CI](https://github.com/arncore/konvoy/actions/workflows/ci.yml/badge.svg)](https://github.com/arncore/konvoy/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/arncore/konvoy/branch/main/graph/badge.svg)](https://codecov.io/gh/arncore/konvoy)

<p align="center">
  <img width="512" height="512" alt="konvoy_logo_under_2mb" src="https://github.com/user-attachments/assets/d300589e-eb0b-4655-a86e-5d936d36e9a4" />
</p>

Konvoy is a native-first Kotlin build tool focused on making Kotlin/Native compilation as easy, fast, and painless as Cargo.

Konvoy avoids Gradle/Maven-style complexity by providing:

- A small, Cargo-like CLI (`konvoy build/test/run`)
- A tiny declarative manifest (`konvoy.toml`)
- Deterministic builds via `konvoy.lock`
- Fast rebuilds via a content-addressed cache
- Predictable output locations under `.konvoy/`

**Scope note:** Konvoy is native-first. JVM builds are intentionally out of scope.

## Table of contents

- [Status](#status)
- [Supported platforms](#supported-platforms)
- [Requirements](#requirements)
- [Quick start](#quick-start)
- [Project layout](#project-layout)
- [Commands](#commands)
- [Output contract](#output-contract)
- [Design goals](#design-goals)
- [Dependencies](#dependencies)
  - [Path dependencies](#path-dependencies)
  - [Maven dependencies](#maven-dependencies)
  - [Plugins](#plugins)
- [Testing](#testing)
- [Managed toolchains](#managed-toolchains)
- [Linting](#linting)
- [Editor support](#editor-support)
- [Development](#development)

## Status

Early-stage prototype / design-driven build. Expect rapid iteration and breaking changes.

## Supported platforms

- **Linux** — x86_64 and ARM64
- **macOS** — x86_64 (Intel) and ARM64 (Apple Silicon)

## Requirements

- **No manual Kotlin/Java installation needed.** Konvoy automatically downloads and manages the Kotlin/Native compiler and a bundled JRE.
- Platform toolchain installed for your host OS:
  - **macOS:** Xcode Command Line Tools (`xcode-select --install`)
  - **Linux:** GCC/build-essential (`sudo apt install build-essential` on Debian/Ubuntu)

## Quick start

### 1) Create a new project

```
konvoy init --name hello
cd hello
```

### 2) Build (host target)

```
konvoy build
```

### 3) Run

```
konvoy run
```

## Project layout

Konvoy projects use:

- `konvoy.toml` — project manifest (name, kind, toolchain, dependencies)
- `konvoy.lock` — pinned toolchain versions and dependency hashes
- `.konvoy/` — build outputs + cache

Example:

```
hello/
  konvoy.toml
  konvoy.lock
  src/
    main.kt
  .konvoy/
    build/
    cache/
```

## Commands

- `konvoy init [--name <name>] [--lib]` — create a new binary or library project
- `konvoy build [--target <triple|host>] [--release] [--verbose] [--force] [--locked]` — compile the project
- `konvoy run [--target <triple|host>] [--release] [--force] [--locked] [-- <args…>]` — build and run
- `konvoy test [--target <triple|host>] [--release] [--verbose] [--force] [--locked] [--filter <pattern>]` — build and run tests
- `konvoy lint [--verbose] [--config <path>] [--locked]` — run detekt static analysis on Kotlin sources
- `konvoy update` — resolve Maven dependencies (including transitives via POM) and update `konvoy.lock`
- `konvoy clean` — remove build artifacts
- `konvoy doctor` — check environment, toolchain, and dependency setup
- `konvoy toolchain install [<version>]` — install a Kotlin/Native version
- `konvoy toolchain list` — list installed toolchain versions

## Output contract

Konvoy writes artifacts to stable paths:

- **Binary debug:** `.konvoy/build/<target>/debug/<name>`
- **Binary release:** `.konvoy/build/<target>/release/<name>`
- **Library debug:** `.konvoy/build/<target>/debug/<name>.klib`
- **Library release:** `.konvoy/build/<target>/release/<name>.klib`

## Design goals

- **No build DSL:** config is declarative; behavior is predictable.
- **Reproducible by default:** lockfile + toolchain fingerprint.
- **Fast inner loop:** content-addressed caching keyed by source+toolchain+target.
- **Actionable errors:** missing toolchain/SDK issues should be one-line fixes.
- **Native-first:** targets are real OS/arch outputs, not JVM bytecode.

## Development

Konvoy is implemented in Rust as a Cargo workspace.

```
cargo build                      # build all crates
cargo test                       # run all tests
cargo run -- build               # run konvoy with a subcommand
cargo test -p konvoy-config      # run tests for a single crate
cargo clippy --workspace         # lint all crates
cargo fmt --all                  # format all crates
```

CI runs check, test (Linux + macOS), clippy, and rustfmt on every push and PR to `main`.

See [docs/code-style.md](docs/code-style.md) for coding conventions.

## Dependencies

Konvoy supports two kinds of dependencies: **path dependencies** (local projects) and **Maven dependencies** (external libraries from Maven Central).

### Path dependencies

Depend on other Konvoy projects via relative paths:

```toml
[dependencies]
my-utils = { path = "../my-utils" }
```

Library projects are created with `konvoy init --lib` and produce `.klib` files. The generated project uses `src/lib.kt` as its entrypoint (instead of `src/main.kt` for binaries):

```
my-utils/
  konvoy.toml
  src/
    lib.kt
```

### Maven dependencies

Depend on external Kotlin/Native libraries from Maven Central using direct Maven coordinates:

```toml
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
kotlinx-coroutines = { maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.8.0" }
kotlinx-datetime = { maven = "org.jetbrains.kotlinx:kotlinx-datetime", version = "0.6.0" }
my-utils = { path = "../my-utils" }
```

The `maven` field is a standard Maven coordinate (`groupId:artifactId`). The `version` field pins the exact version to use.

Each dependency must have exactly one source type — either `path` or `maven` + `version` — not both.

#### Workflow

After adding or changing a Maven dependency, run `konvoy update` to resolve and lock:

```
konvoy update    # resolve deps, fetch POMs, write hashes to konvoy.lock
konvoy build     # downloads only the klib needed for your host target
```

`konvoy update` performs these steps:

1. Reads `[dependencies]` from `konvoy.toml`
2. Fetches artifact metadata (`.module` JSON first, POM XML as fallback) from Maven Central
3. Resolves transitive dependencies via BFS with cycle detection
4. Detects version conflicts (suggests pinning an explicit version in `konvoy.toml`)
5. Downloads the per-target `.klib` for each supported platform and computes SHA-256 hashes (also discovers cinterop klibs from `.module` metadata)
6. Writes the full dependency set to `konvoy.lock` with `required_by` for transitive deps

At build time (`konvoy build`), only the klib for your current host target is needed. Subsequent builds reuse cached artifacts from `~/.konvoy/cache/maven/`.

#### Lockfile

`konvoy.lock` pins the exact version and per-target SHA-256 hash for every Maven dependency (direct and transitive). Transitive dependencies include a `required-by` field tracing the dependency chain back to `konvoy.toml`:

```toml
[[dependencies]]
name = "kotlinx-coroutines"
source_type = "maven"
version = "1.8.0"
maven = "org.jetbrains.kotlinx:kotlinx-coroutines-core"
source_hash = "..."

[dependencies.targets]
linux_x64 = "sha256:..."
macos_arm64 = "sha256:..."
```

Transitive dependencies are tracked automatically with a `required_by` field listing which direct dependency pulled them in.

Use `--locked` on build/test/run to error if the lockfile is out of date instead of silently updating.

### Plugins

Konvoy supports compiler plugins via the `[plugins]` section. Plugins use Maven coordinates — any Kotlin/Native compiler plugin JAR on Maven Central can be used:

```toml
[plugins]
serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
```

The `{kotlin}` placeholder resolves to the Kotlin version set in `[toolchain]`. Runtime libraries needed by the plugin (e.g., `kotlinx-serialization-core`, `kotlinx-serialization-json`) should be added as regular Maven dependencies in `[dependencies]`:

```toml
[dependencies]
kotlinx-serialization-core = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-core", version = "1.7.3" }
kotlinx-serialization-json = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-json", version = "1.7.3" }
```

## Testing

Konvoy has a built-in test framework using `kotlin.test`. Test sources live in `src/test/` and are compiled alongside your project sources using konanc's `-generate-test-runner` flag.

### Writing tests

Create test files under `src/test/`:

```
hello/
  src/
    main.kt
    test/
      math_test.kt
```

Tests use standard `kotlin.test` annotations:

```kotlin
import kotlin.test.Test
import kotlin.test.assertEquals

class MathTest {
    @Test
    fun addition() {
        assertEquals(4, 2 + 2)
    }
}
```

### Running tests

```
konvoy test
```

Filter tests by name pattern:

```
konvoy test --filter "MathTest.*"
```

The `--filter` flag is forwarded to the test runner as `--ktest_filter`.

Test builds are cached separately from regular builds (using a `debug-test` / `release-test` profile key), so running `konvoy test` won't invalidate your normal build cache.

## Managed toolchains

Konvoy automatically downloads and manages Kotlin/Native toolchains. The first `konvoy build` (or `konvoy toolchain install`) downloads the compiler and a bundled JRE to `~/.konvoy/toolchains/<version>/`. No manual Kotlin or Java installation is required.

## Linting

Konvoy integrates [detekt](https://detekt.dev) for Kotlin static analysis. Enable it by adding `detekt` to `[toolchain]` in `konvoy.toml`:

```toml
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"
detekt = "1.23.7"
```

The detekt-cli JAR is automatically downloaded to `~/.konvoy/tools/detekt/<version>/` on first use and its SHA-256 hash is pinned in `konvoy.lock`.

Detekt runs using the JRE bundled with the managed Kotlin/Native toolchain, so no separate Java installation is needed.

To customize detekt rules, place a `detekt.yml` file in the project root or pass `--config <path>`:

```
konvoy lint                        # run with defaults or detekt.yml
konvoy lint --config my-rules.yml  # use custom config
konvoy lint --verbose              # show raw detekt output
```

## Editor support

### VS Code

Install [Konvoy for VS Code](https://marketplace.visualstudio.com/items?itemName=konvoy.konvoy-vscode) from the Marketplace, or search "Konvoy" in the Extensions panel.

- **Commands** — Build, Run, Test, Lint, Clean, Doctor, Update, and Toolchain management via `Ctrl+Shift+P`
- **Title bar buttons** — Build (dropdown), Run (debug/release toggle), Test, Lint, Clean, Update, and Doctor in the editor title bar for `.kt` files, `konvoy.toml`, and `konvoy.lock`
- **`konvoy.toml` support** — Syntax highlighting, validation on save, autocomplete, and hover docs
- **Diagnostics** — Build errors and detekt findings in the Problems panel
- **Tasks** — Auto-detected konvoy tasks via `Ctrl+Shift+B`

See the [extension README](editors/code/README.md) for full details.

### IntelliJ IDEA

The Konvoy IntelliJ plugin provides full Kotlin/Native language intelligence by teaching IntelliJ how to read `konvoy.toml`. Once installed, IntelliJ's built-in Kotlin plugin handles completion, navigation, refactoring, and diagnostics automatically.

**Features:**

- **Project sync** — Parses `konvoy.toml` and `konvoy.lock` to configure modules, source roots, klib dependencies, and Kotlin/Native target platform
- **Auto re-sync** — Watches `konvoy.toml` and `konvoy.lock` for changes
- **Run configurations** — Build, Run, Test, and Lint via the standard run menu
- **Toolchain SDK** — Discovers managed toolchains from `~/.konvoy/toolchains/`
- **Full Kotlin intelligence** — Completion, go-to-definition, find usages, rename, diagnostics, and refactoring (provided by IntelliJ's Kotlin plugin)

**Install from source:**

```bash
cd editors/intellij
./gradlew buildPlugin
```

The plugin zip will be at `build/distributions/konvoy-intellij-<version>.zip`. Install it in IntelliJ via **Settings → Plugins → ⚙️ → Install Plugin from Disk...** and select the zip file.

**Requirements:** IntelliJ IDEA 2024.2+ (Community or Ultimate) with the Kotlin plugin installed.

