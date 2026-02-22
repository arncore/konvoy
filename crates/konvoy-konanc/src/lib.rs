//! Compiler invocation and diagnostics normalization for `konanc`.

pub mod detect;
pub mod error;
pub mod invoke;

pub use detect::{detect_konanc, KonancInfo};
pub use error::KonancError;
pub use invoke::{CompilationResult, Diagnostic, DiagnosticLevel, KonancCommand};
