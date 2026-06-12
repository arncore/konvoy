#![forbid(unsafe_code)]
//! Build orchestration, cache keying, and artifact store for Konvoy.

pub mod artifact;
pub mod build;
pub mod cache;
pub mod codegen;
mod common;
pub mod detekt;
mod diagnostics;
pub mod error;
pub mod init;
pub mod managed_tool;
pub mod plugin;
pub mod resolve;
pub mod test_build;
pub mod update;

pub use artifact::{ArtifactStore, BuildMetadata};
pub use build::{build, BuildOptions, BuildOutcome, BuildResult};
pub use cache::{CacheInputs, CacheKey};
pub use codegen::{
    compute_codegen_hash_pairs, compute_codegen_hashes, generator_output_dir, generator_summaries,
    managed_tools, CodeGenerator, GeneratorSummary,
};
pub use common::ArtifactResolver;
pub use detekt::{lint, DetektDiagnostic, LintOptions, LintResult};
pub use error::EngineError;
pub use init::{
    init_project, init_project_in_place, init_project_with_kind, DEFAULT_KOTLIN_VERSION,
};
pub use managed_tool::{ManagedToolSpec, ToolOutput, ToolRuntime, ToolSource};
pub use plugin::{
    ensure_plugin_artifacts, resolve_plugin_artifacts, PluginArtifactResult, ResolvedPluginArtifact,
};
pub use resolve::{resolve_dependencies, ResolvedGraph};
pub use test_build::{build_tests, TestBuildResult};
pub use update::{update, UpdateResult};
