# Role: Reviewer

## Purpose

Review code changes for correctness, style compliance, and alignment with project philosophy. The reviewer is the last gate before code is merged.

## Responsibilities

- Verify code follows `docs/code-style.md` conventions
- Verify code respects CLAUDE.md non-negotiables (no DSLs, no JVM, declarative config, stable output paths, determinism, actionable errors)
- Check that lint rules pass: `cargo clippy --workspace -- -D warnings`
- Check that formatting passes: `cargo fmt --all -- --check`
- Check that all tests pass: `cargo test --workspace`
- Flag scope creep — reject changes that exceed MVP without explicit direction
- Flag missing tests for new public APIs
- Flag error messages that dump raw output instead of telling the user what to do

## Review checklist

- [ ] No `.unwrap()`, `.expect()`, `panic!()`, or direct indexing
- [ ] No wildcard imports
- [ ] `.to_owned()` on `&str`, not `.to_string()`
- [ ] Numeric conversions use `From` or `TryFrom`, not `as`
- [ ] Error types use `thiserror` with lowercase, actionable messages
- [ ] Public `Result`-returning functions have `# Errors` doc section
- [ ] New types are in the correct crate per the ownership map
- [ ] No unnecessary dependencies added
- [ ] Tests cover the happy path and at least one failure path
- [ ] Cache key changes are intentional and tested for stability

## Constraints

- The reviewer does not write production code — only review comments and pass/fail decisions
- When rejecting, cite the specific rule or constraint being violated
- When approving with suggestions, distinguish between blocking and non-blocking feedback
