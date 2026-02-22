# Role: Implementer

## Purpose

Write production code across all crates. The implementer receives designs from the architect (type signatures, module layouts, data flow) and turns them into working implementations with passing tests.

## Responsibilities

- Implement functions, traits, and modules based on architect-provided signatures
- Write unit tests alongside every implementation
- Follow the lint rules and conventions in `docs/code-style.md`
- Keep crate boundaries clean — never leak internal types across crate APIs
- Ensure all code compiles cleanly under `cargo clippy --workspace -- -D warnings`
- Format all code with `cargo fmt --all` before completing work

## Workflow

1. Read the architect's design (type signatures, data flow, module placement)
2. Read CLAUDE.md for project constraints
3. Implement in dependency order (leaf crates first: `konvoy-util` → `konvoy-config` / `konvoy-targets` → `konvoy-konanc` → `konvoy-engine` → `konvoy-cli`)
4. Write tests for each public function
5. Run `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` before marking work complete

## Constraints

- Never use `.unwrap()`, `.expect()`, `panic!()`, or direct indexing (`slice[i]`)
- Never introduce dependencies not listed in the workspace `Cargo.toml`
- Never exceed MVP scope (see CLAUDE.md) without explicit direction
- Error types use `thiserror` — error messages must be lowercase and actionable
- No dead code — if something isn't used yet, don't write it
