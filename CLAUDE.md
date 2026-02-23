# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

Konvoy is a native-first Kotlin build tool implemented in Rust. The goal is to make Kotlin/Native builds feel like Cargo: simple commands, deterministic outputs, and fast rebuilds via caching.

## Non-negotiables (project philosophy)

- **No Gradle/Maven concepts:** do not introduce task graphs, build DSLs, or plugin scripting.
- **Native-first only:** no JVM support in scope.
- **Declarative configuration:** behavior must be driven by `konvoy.toml` + `konvoy.lock`.
- **Stable output paths:** do not change the output contract casually.
- **Determinism first:** if something can vary machine-to-machine, it must be pinned or explicitly detected and reported.
- **Actionable diagnostics:** errors must tell users what to do, not dump raw compiler output (unless `--verbose`).

## Project file conventions

- **Manifest:** `konvoy.toml`
- **Lockfile:** `konvoy.lock`
- **Local project dir:** `.konvoy/`
- **Outputs:**
  - `.konvoy/build/<target>/debug/<name>` (binary)
  - `.konvoy/build/<target>/release/<name>` (binary)
  - `.konvoy/build/<target>/debug/<name>.klib` (library)
  - `.konvoy/build/<target>/release/<name>.klib` (library)
- **Managed toolchains:** `~/.konvoy/toolchains/<version>/` (konanc + bundled JRE)
- **Managed tools:** `~/.konvoy/tools/detekt/<version>/` (detekt-cli JAR)

## Current scope

- Target support: host only
- Output kinds: native executable (`bin`) and Kotlin/Native library (`lib` / `.klib`)
- Compiler: invoke `konanc` as an external process via managed toolchains
- Managed toolchains: automatic download of Kotlin/Native + bundled JRE to `~/.konvoy/toolchains/`
- Commands: `init`, `build`, `run`, `test`, `lint`, `clean`, `doctor`, `toolchain install/list`
- Dependencies: path-based (`{ path = "..." }`); no external registry
- No plugin system
- No C interop beyond detection placeholders

## Implementation structure (recommended crates)

- `konvoy-cli`: CLI parsing and command routing
- `konvoy-config`: parse/validate `konvoy.toml` and `konvoy.lock`
- `konvoy-targets`: host detection + target mapping
- `konvoy-engine`: cache keying, artifact store, build orchestration
- `konvoy-konanc`: compiler invocation and diagnostics normalization
- `konvoy-util`: hashing, filesystem utilities, process helpers

If the repo currently has fewer crates, keep code modular with clear boundaries so extraction is easy.

## Build commands

```
cargo build
cargo test
cargo run -- <konvoy-subcommand>   # e.g. cargo run -- build
cargo test -p konvoy-config        # run tests for a single crate
```

## Cache key rules (critical)

Cache keys MUST include:
- normalized `konvoy.toml`
- relevant `konvoy.lock` content
- `konanc` version and a fingerprint of the `konanc` binary
- target + profile flags
- all source file contents (`src/**/*.kt` at minimum)
- explicit environment identifiers (OS/arch)

Cache behavior:
- cache is immutable (content-addressed)
- build outputs are "materialized" views (hardlink preferred, else copy)

## Diagnostics rules

- If `konanc` is missing: emit a single-line fix (env var/path hint).
- If platform toolchain missing (e.g., Xcode CLT): emit the canonical install command.
- Only show raw `konanc` output under `--verbose`.

## When uncertain

- Prefer the simplest behavior consistent with the philosophy.
- If a feature adds configuration complexity, defer it.
- If something requires scripting, reject it and propose a declarative alternative.

## Contribution style

- Keep functions small and testable.
- Add unit tests for parsing, hashing, target mapping, and cache-key stability.
- Avoid cleverness: readability and determinism > micro-optimizations.
- Document behavior in code comments where it prevents future Gradle-like sprawl.
