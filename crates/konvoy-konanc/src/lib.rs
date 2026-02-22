//! Compiler invocation and diagnostics normalization for `konanc`.

pub mod detect;
pub mod invoke;

pub use detect::{detect_konanc, KonancInfo};
