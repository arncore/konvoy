#![forbid(unsafe_code)]
//! Build orchestration, cache keying, and artifact store for Konvoy.

pub mod artifact;
pub mod build;
pub mod cache;
pub mod detekt;
mod diagnostics;
pub mod error;
pub mod init;
pub mod resolve;
pub mod test_build;

pub use artifact::{ArtifactStore, BuildMetadata};
pub use build::{build, BuildOptions, BuildOutcome, BuildResult};
pub use cache::{CacheInputs, CacheKey};
pub use detekt::{lint, DetektDiagnostic, LintOptions, LintResult};
pub use error::EngineError;
pub use init::{init_project, init_project_in_place, init_project_with_kind};
pub use resolve::{resolve_dependencies, ResolvedGraph};
pub use test_build::{build_tests, TestBuildResult, TestOptions};
