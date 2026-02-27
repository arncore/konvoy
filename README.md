# Konvoy

Konvoy is a native-first Kotlin build tool focused on making Kotlin/Native compilation as easy, fast, and painless as Cargo.

Konvoy avoids Gradle/Maven-style complexity by providing:

- A small, Cargo-like CLI (`konvoy build/test/run`)
- A tiny declarative manifest (`konvoy.toml`)
- Deterministic builds via `konvoy.lock`
- Fast rebuilds via a content-addressed cache
- Predictable output locations under `.konvoy/`

**Scope note:** Konvoy is native-first. JVM builds are intentionally out of scope.

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
- `konvoy update` — resolve Maven dependencies and update `konvoy.lock`
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

Depend on external Kotlin/Native libraries from Maven Central using a curated library index:

```toml
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
kotlinx-coroutines = { version = "1.8.0" }
kotlinx-datetime = { version = "0.6.0" }
my-utils = { path = "../my-utils" }
```

After adding a Maven dependency, run `konvoy update` to resolve versions and populate `konvoy.lock` with per-target SHA-256 hashes. At build time, only the klib for your current target is downloaded — subsequent builds use the cached artifact.

```
konvoy update    # resolve deps, download klibs, write hashes to konvoy.lock
konvoy build     # downloads only the klib needed for your host target
```

Each dependency must have exactly one of `path` or `version` — not both.

### Available libraries

Konvoy ships with a curated index of popular Kotlin/Native libraries:

| Name | Maven artifact |
|------|---------------|
| `kotlinx-coroutines` | `org.jetbrains.kotlinx:kotlinx-coroutines-core` |
| `kotlinx-datetime` | `org.jetbrains.kotlinx:kotlinx-datetime` |
| `kotlinx-io` | `org.jetbrains.kotlinx:kotlinx-io-core` |
| `kotlinx-atomicfu` | `org.jetbrains.kotlinx:atomicfu` |

Run `konvoy doctor` to see the full list of available libraries.

### Plugins

Konvoy supports compiler plugins via the `[plugins]` section. Plugins are data-driven — no scripting or build DSL:

```toml
[plugins]
serialization = { version = "2.1.0" }
```

Plugins are resolved alongside toolchain downloads. The `serialization` plugin automatically includes the `core` runtime module; additional modules (e.g., `json`, `cbor`) can be enabled:

```toml
[plugins]
serialization = { version = "2.1.0", modules = ["json"] }
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

## Roadmap (high level)

1. ~~**MVP:** host-native executable build/run + cache~~ done
2. ~~**Tests:** built-in test framework using `kotlin.test`~~ done
3. ~~**Targets:** explicit target triples~~ done
4. **Dependencies:** ~~path~~ done → ~~Maven Central~~ done → git → url+sha → registry
5. ~~**Toolchain install/pinning**~~ done
6. ~~**Linting:** detekt integration~~ done
7. ~~**Plugins:** data-driven compiler plugins (serialization)~~ done
8. **Remote cache** (later)
