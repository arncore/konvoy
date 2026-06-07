# Codegen smoke tests — design / spec

**Date:** 2026-06-07
**Status:** Approved
**Scope:** Add real, end-to-end smoke-test coverage for the OpenAPI codegen feature (PR #280) to the existing smoke harness.

## Goal

Cover **expected usage** of `[codegen.openapi]` end-to-end with the real `konvoy`
binary and a real toolchain (konanc + JRE + Fabrikt + Maven), including full
compilation of generated sources. This is the user-selected scope: real
end-to-end, full compile, opt-in (not in normal CI).

## Where this lives (decision)

Extend the **existing smoke harness**, do not invent a new one:

- `tests/smoke/tests.sh` — bash suite. Each `test_*` function runs in a fresh
  temp dir, calls the `konvoy` binary, uses the existing assert helpers, and is
  registered with `run_test <name>` near the bottom. Tests run in parallel
  (`MAX_PARALLEL`).
- `tests/smoke/Dockerfile` — builds konvoy, pre-installs toolchain `2.2.0`, warms
  the konanc cache.
- `tests/smoke/run.sh` — local runner (`bash tests/smoke/run.sh`).
- CI: `.github/workflows/ci.yml` `smoke` job, manual trigger only
  (`workflow_dispatch` with `smoke=true`). **No CI gating changes needed** — the
  opt-in nature is already provided by the harness.

Rejected alternatives: a separate Rust `#[ignore]` e2e suite, or a standalone
script — both fragment the established harness.

## Assert helpers available (already in tests.sh)

`assert_contains`, `assert_not_contains`, `assert_file_exists`,
`assert_dir_exists`, `assert_dir_not_exists`, `assert_file_contains`,
`assert_file_not_contains`. Tests `cd` into a per-test temp dir automatically.

## Conventions to follow

- Combine lifecycle steps into one function to minimize expensive konanc/Fabrikt
  invocations (mirror `test_build_lifecycle`, `test_maven_dep_build_lifecycle`).
- Kotlin version: `2.2.0` (the version warmed in the Dockerfile).
- Fabrikt version: `20.0.0`.
- `main.kt` stays trivial (`fun main() { println("ok") }`). Generated models are
  compiled **alongside** it, so "do they compile" is proven without hard-coding
  Fabrikt's output package/class names.
- Write fixtures inline with heredocs/`printf` (existing style).

## Exact strings to assert against (from the implementation)

| Concept | String |
|---|---|
| Generated dir | `.konvoy/gen/openapi` |
| Input-hash file | `.konvoy/gen/openapi/.input_hash` |
| Lockfile pin header | `[codegen_tools.fabrikt]` |
| Generated content marker | `@Serializable` |
| Generate progress line | `Generating OpenAPI sources with Fabrikt` |
| Build cache hit | `(cached)` |
| Compile line | `Compiling` |
| `--locked` refusal | `lockfile is out of date` |
| doctor tool ok | `[ok] fabrikt:` |
| doctor pin ok | `[ok] Lockfile pin: fabrikt` |
| doctor all-clear | `All checks passed` |
| version below floor | `18.0.0 or newer` |
| non-numeric version | `is not a valid Fabrikt version` |
| absolute spec | `spec must be a relative path inside the project` |
| `..` in spec | `must not contain` |
| bad spec extension | `.yaml` (from "spec must point to an OpenAPI .yaml, .yml, or .json file") |
| missing spec file | `not found at` (from "codegen input for `openapi` not found at …") |
| no codegen configured | `no codegen configured` |

## Fixtures

**Minimal single-file spec** (`specs/api.yaml`):

```yaml
openapi: 3.0.3
info:
  title: Pet API
  version: 1.0.0
paths: {}
components:
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties:
        id:
          type: integer
          format: int64
        name:
          type: string
```

**Multi-file `$ref` spec** for the ref-tracking test:

- `specs/api.yaml` whose `components.schemas.Pet` is `{ $ref: './pet.yaml#/Pet' }`
- `specs/pet.yaml` containing the `Pet` schema (edited in the test to prove
  the sub-file feeds the hash).

**Manifest for the full-compile build test** (`konvoy.toml`):

```toml
[package]
name = "codegen-build"

[toolchain]
kotlin = "2.2.0"

[codegen.openapi]
version = "20.0.0"
spec = "specs/api.yaml"
base_package = "com.example.api"

[dependencies]
kotlinx-serialization-core = { maven = "org.jetbrains.kotlinx:kotlinx-serialization-core", version = "1.7.3" }

[plugins]
serialization = { maven = "org.jetbrains.kotlin:kotlin-serialization-compiler-plugin", version = "{kotlin}" }
```

`src/main.kt`: `fun main() { println("ok") }`

## Tests to add

Each maps to expected usage and (where noted) validates a specific fix from the
review.

1. **`test_codegen_generate_lifecycle`** *(JRE + Fabrikt; no konanc compile)*
   - Project with `[codegen.openapi]` + single-file spec, no deps/plugins.
   - `konvoy generate` → output contains `Generating OpenAPI sources with Fabrikt`.
   - `.konvoy/gen/openapi` exists and contains at least one `.kt` file.
   - A generated `.kt` file contains `@Serializable`.
   - `.konvoy/gen/openapi/.input_hash` exists.
   - `konvoy.lock` contains `[codegen_tools.fabrikt]`.
   - Re-run `konvoy generate`: idempotent (still succeeds; pin unchanged).
   - `konvoy generate --force`: regenerates (delete a generated file first, then
     assert it reappears after `--force`).
   - Edit the spec content (add a second schema), `konvoy generate`: a new model
     file appears OR the generated set changes (assert a file mentioning the new
     schema name exists).

2. **`test_codegen_build_lifecycle`** *(full compile — konanc + JRE + Fabrikt + Maven; the heavy one)*
   - Full manifest above (codegen + serialization dep + serialization plugin) +
     trivial `main.kt`.
   - `konvoy build` → output contains `Compiling`; binary exists at
     `.konvoy/build/linux_x64/debug/codegen-build`; generated `@Serializable`
     models compiled in (build succeeds).
   - Second `konvoy build` → output contains `(cached)` and **not** `Compiling`
     **(validates M1: no spurious rebuild after codegen).**
   - `touch`/edit `specs/api.yaml`, `konvoy build` → recompiles (`Compiling`).

3. **`test_codegen_ref_change_regenerates`** *(JRE + Fabrikt)* **(validates M3)**
   - Multi-file `$ref` spec. `konvoy generate` once.
   - Edit **only** `specs/pet.yaml` (add a property, e.g. `tag`).
   - `konvoy generate` again → the regenerated output reflects the change
     (assert a generated file now contains `tag`).

4. **`test_codegen_locked_requires_pin`** *(JRE + Fabrikt)*
   - Fresh project, `konvoy generate --locked` → fails, stderr contains
     `lockfile is out of date`.
   - Plain `konvoy generate` (pins fabrikt), then `konvoy generate --locked`
     → succeeds.

5. **`test_codegen_doctor_reports_pin`** *(JRE + Fabrikt)* **(validates L4)**
   - After a `konvoy generate`, `konvoy doctor` output contains `[ok] fabrikt:`
     and `[ok] Lockfile pin: fabrikt`.
   - (Optional) Before any generate, doctor reports the missing pin
     (`[!!] Lockfile pin:`), confirming the gap is surfaced.

6. **`test_codegen_invalid_version_rejected`** *(fast — fails at manifest validation, no downloads)* **(validates M4)**
   - `version = "17.0.0"` → `konvoy generate` fails, stderr contains
     `18.0.0 or newer`.
   - `version = "latest"` → fails, stderr contains `is not a valid Fabrikt version`.

7. **`test_codegen_invalid_spec_rejected`** *(fast)* **(validates L1)**
   - `spec = "/etc/api.yaml"` → fails, stderr contains
     `spec must be a relative path inside the project`.
   - `spec = "../api.yaml"` → fails, stderr contains `must not contain`.
   - `spec = "specs/api.txt"` → fails, stderr contains `.yaml`.

8. **`test_codegen_missing_spec_errors`** *(fast)*
   - `[codegen.openapi]` pointing at a nonexistent `specs/api.yaml` →
     `konvoy generate` fails, stderr contains `not found at`.

Register all eight with `run_test` in a new "Tests: codegen (OpenAPI)" section of
the runner block.

## Dockerfile warmup (consistency + speed)

Add a warmup step (after the konanc warmup) that pre-downloads the Fabrikt JAR and
serialization runtime so the heavy build test isn't a cold-cache outlier and the
install path is itself smoke-tested. Build a throwaway codegen project and run
`konvoy build` on it, then remove it. This pre-populates
`~/.konvoy/tools/fabrikt/20.0.0/`, the kotlinx-serialization klib cache, and the
serialization plugin jar.

## Known risk (the build test is designed to surface it)

Fabrikt `--targets HTTP_MODELS --serialization-library KOTLINX_SERIALIZATION`
output must compile with only `kotlinx-serialization-core` + the serialization
compiler plugin. If Fabrikt emits validation/jackson imports, the build test will
fail loudly — valuable signal. The required dependency set is documented in the
manifest fixture above; adjust if the smoke run reveals additional needs.

## Verification plan

- `bash -n tests/smoke/tests.sh` (syntax) and `shellcheck` if available.
- Every new `test_*` has a matching `run_test` line; names unique.
- Fast subset (tests 6–8) runs against a locally built debug `konvoy` binary
  **without Docker** (they fail at validation before any download) — run these
  directly as partial local verification.
- Full suite runs via `bash tests/smoke/run.sh` (Docker) — documented; not run in
  this environment if Docker is unavailable.
