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
- `konvoy build [--target <triple|host>] [--release] [--verbose]` — compile the project
- `konvoy run [--target <triple|host>] [--release] [-- <args…>]` — build and run
- `konvoy test [--target <triple|host>] [--release] [--verbose]` — build and run as test
- `konvoy lint [--verbose] [--config <path>]` — run detekt static analysis on Kotlin sources
- `konvoy clean` — remove build artifacts
- `konvoy doctor` — check environment and toolchain setup
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

Library projects can depend on other Konvoy projects via path dependencies:

```toml
[package]
name = "my-app"

[toolchain]
kotlin = "2.1.0"

[dependencies]
my-utils = { path = "../my-utils" }
```

Library projects are created with `konvoy init --lib` and produce `.klib` files.

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
2. ~~**Tests:** minimal native test runner model~~ done
3. ~~**Targets:** explicit target triples~~ done
4. **Dependencies:** ~~path~~ done → git → url+sha → registry
5. ~~**Toolchain install/pinning**~~ done
6. ~~**Linting:** detekt integration~~ done
7. **Remote cache** (later)
