# Workflow

How agents coordinate to avoid conflicts and maximize parallel throughput.

## Dependency layers

Issues are grouped into layers. All issues within a layer can run in parallel. A layer cannot start until the layer above it is merged.

```
Layer 0 (foundation) ─ no internal deps, fully parallel
  #1  konvoy-util: hashing
  #2  konvoy-util: fs + process helpers
  #3  konvoy-config: manifest parsing
  #4  konvoy-config: lockfile parsing
  #5  konvoy-targets: host detection

Layer 1 (compiler) ─ depends on Layer 0
  #6  konvoy-konanc: detection + version
  #7  konvoy-konanc: invocation + diagnostics

Layer 2 (engine) ─ depends on Layers 0 + 1
  #8  konvoy-engine: cache key computation
  #9  konvoy-engine: artifact store
  #10 konvoy-engine: build orchestration (depends on #8, #9)
  #11 konvoy-engine: init scaffolding

Layer 3 (CLI) ─ depends on Layers 0 + 1 + 2
  #12 konvoy-cli: init
  #13 konvoy-cli: build
  #14 konvoy-cli: run
  #15 konvoy-cli: test
  #16 konvoy-cli: clean
  #17 konvoy-cli: doctor
```

## Crate ownership rule

Two agents must never edit the same crate at the same time. Within a layer, parallel work is safe because each issue targets a different crate or a different module within a crate.

Safe parallel pairs (no file overlap):
- #1 (util/hash) + #3 (config/manifest) + #5 (targets) — three different crates
- #1 (util/hash) + #2 (util/fs+process) — same crate but different modules (`hash.rs` vs `fs.rs`/`process.rs`)
- #3 (config/manifest) + #4 (config/lockfile) — same crate but different modules (`manifest.rs` vs `lockfile.rs`)

Unsafe (must be sequential):
- Any two issues that modify the same file (e.g., both touching `lib.rs` re-exports)
- #10 depends on #8 and #9 within the same layer

## Agent assignments

| Role | What they do | When |
|---|---|---|
| **Architect** | Designs API signatures for the current layer | Before implementers start a new layer |
| **Implementer** (×N) | Implements one issue at a time in an isolated worktree | After architect approves the layer's API |
| **Tester** | Adds tests to completed implementations | After an implementer finishes an issue |
| **Reviewer** | Runs checklist, approves or rejects | After tester confirms tests pass |

## Branching strategy

Each issue gets its own branch:
- Branch name: `issue-<number>-<short-description>` (e.g., `issue-1-hashing-utils`)
- Agents work in isolated git worktrees to avoid file conflicts
- Branches are merged to `main` only after reviewer approval
- Rebase onto `main` before merging to keep history linear

## Handoff protocol

1. **Architect → Implementers:** Architect posts API design (type signatures, module layout) as a message. Implementers claim issues by assignment.

2. **Implementer → Tester:** Implementer finishes code + basic tests, marks issue as ready. Tester picks it up and adds edge case / invariant tests.

3. **Tester → Reviewer:** Tester confirms `cargo test -p <crate>` passes. Reviewer runs the full checklist from `docs/agents/reviewer.md`.

4. **Reviewer → Merge:** Reviewer approves. Lead merges the branch to `main`.

## Issue assignment (deadlock prevention)

Before writing any code, an agent MUST assign the GitHub issue to themselves:
```
gh issue edit <number> --add-assignee @me
```

Rules:
- **Check before claiming.** If an issue already has an assignee, do not work on it. Pick a different unassigned issue from the same layer.
- **Assign atomically.** Assign the issue immediately when you start, not after you've begun coding.
- **Unassign on abort.** If you cannot complete an issue, unassign yourself so another agent can pick it up:
  ```
  gh issue edit <number> --remove-assignee @me
  ```
- **One issue per agent.** An agent works on exactly one issue at a time. Finish or release the current issue before claiming another.
- **Check crate conflicts.** Before claiming, verify no other assigned issue in the same layer touches the same crate files. If it does, wait for that issue to merge first.

This prevents two agents from unknowingly working on overlapping code, which causes merge conflicts and wasted work.

## Conflict prevention rules

1. **Claim before starting.** An agent must be assigned to an issue before writing code. No two agents work the same issue.
2. **One crate, one writer.** If two issues touch the same crate, only one implementer works at a time unless they are in strictly separate modules (different `.rs` files with no shared `lib.rs` changes).
3. **lib.rs is a bottleneck.** Any change to a crate's `lib.rs` (adding `pub mod`, re-exports) must happen in the implementer's branch. If two branches both need `lib.rs` changes, the second one rebases after the first merges.
4. **Cargo.toml is shared.** Changes to workspace `Cargo.toml` (new dependencies) must be coordinated through the lead. One branch at a time.
5. **Merge fast.** Small, focused PRs. Don't batch multiple issues into one branch.

## Parallelism targets

- **Layer 0:** Up to 5 agents in parallel (one per issue, or 3 if grouping by crate)
- **Layer 1:** Up to 2 agents in parallel
- **Layer 2:** Up to 3 agents in parallel (#8, #9, #11 are independent; #10 waits for #8 + #9)
- **Layer 3:** Up to 6 agents in parallel (all CLI commands are independent once engine is done)

## Completion criteria

A layer is complete when:
- All issues in the layer are merged to `main`
- `cargo build` compiles cleanly
- `cargo test --workspace` passes
- `cargo clippy --workspace -- -D warnings` is clean
- `cargo fmt --all -- --check` passes

Only then does the next layer begin.
