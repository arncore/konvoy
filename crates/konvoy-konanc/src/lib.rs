//! Compiler invocation and diagnostics normalization for `konanc`.

pub mod detect;
pub mod error;
pub mod invoke;
pub mod toolchain;

pub use detect::{resolve_konanc, KonancInfo, ResolvedKonanc};
pub use error::KonancError;
pub use invoke::{CompilationResult, Diagnostic, DiagnosticLevel, KonancCommand};
