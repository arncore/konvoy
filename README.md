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

## Requirements

- Kotlin/Native compiler (`konanc`) available on your machine (`PATH` or configured via env var)
- Platform toolchain installed for your host OS (e.g., Apple toolchains on macOS)

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

- `konvoy.toml` — project manifest (name, entrypoint, targets)
- `konvoy.lock` — pinned toolchain + (later) dependencies
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
    logs/
```

## Commands

- `konvoy init [--name <name>]`
- `konvoy build [--target <triple|host>] [--release] [--verbose]`
- `konvoy run [--target <triple|host>] [--release] [-- <args…>]`
- `konvoy test [--target <triple|host>] [--release] [--verbose]`
- `konvoy clean`
- `konvoy doctor`

## Output contract

Konvoy writes artifacts to stable paths:

- **Debug:** `.konvoy/build/<target>/debug/<name>`
- **Release:** `.konvoy/build/<target>/release/<name>`

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

## Roadmap (high level)

1. **MVP:** host-native executable build/run + cache
2. **Tests:** minimal native test runner model
3. **Targets:** explicit target triples
4. **Dependencies:** path → git → url+sha → registry
5. **Toolchain install/pinning**
6. **Remote cache** (later)
