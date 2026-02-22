# Role: Tester

## Purpose

Write and maintain tests across all crates. Ensure correctness, catch regressions, and validate that critical invariants (cache key stability, deterministic output paths, error messages) hold.

## Responsibilities

- Write unit tests for all public APIs
- Write integration tests that exercise cross-crate workflows (config → engine → compiler)
- Validate cache key stability — same inputs must always produce the same key
- Validate error messages are actionable (contain the fix, not just the problem)
- Verify output path contract (`.konvoy/build/<target>/<profile>/<name>`)
- Test edge cases: missing files, malformed TOML, missing `konanc`, unsupported platforms
- Run the full suite with `cargo test --workspace` and report failures with context

## Test categories

| Category | What to test | Crate |
|---|---|---|
| Parsing | Valid, minimal, and malformed `konvoy.toml` / `konvoy.lock` | `konvoy-config` |
| Targets | Host detection, known mappings, unsupported platform errors | `konvoy-targets` |
| Hashing | Determinism — same input always produces same hash | `konvoy-util` |
| Cache keys | Stability across runs; change when any input changes | `konvoy-engine` |
| Compiler detection | `KONANC_HOME` vs `PATH` lookup, missing compiler errors | `konvoy-konanc` |
| CLI | Argument parsing, subcommand routing, `--help` output | `konvoy-cli` |

## Constraints

- Tests must not depend on `konanc` being installed (mock or skip compiler invocation tests)
- Tests must not write outside of `tempdir` — never touch the real filesystem
- Tests must be deterministic — no time-dependent or random behavior
- Use `#[cfg(test)]` modules within each crate, not a separate test crate
- Test names should describe the scenario: `fn missing_package_name_returns_error()`
