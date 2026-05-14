//! Test-only helpers shared across modules in the `konvoy-util` crate.
//!
//! Currently exposes a single process-wide mutex used to serialize tests
//! that read or mutate `HOME` / `USERPROFILE`. Tests in different modules
//! (`fs`, `pom`, `module_metadata`) all touch the same env vars, so they
//! must share a single guard or they will race when `cargo test` runs them
//! on multiple threads in the same binary.

/// Guards tests that read or mutate HOME / USERPROFILE env vars so they
/// don't race with each other (env vars are process-wide shared state).
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
