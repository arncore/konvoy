//! Shared diagnostic printing for build and test pipelines.

use konvoy_konanc::invoke::{CompilationResult, DiagnosticLevel};

/// Print structured diagnostics from a compilation result to stderr.
///
/// When `verbose` is true, raw compiler stdout/stderr is also printed.
pub(crate) fn print_diagnostics(result: &CompilationResult, verbose: bool) {
    for diag in &result.diagnostics {
        let prefix = match diag.level {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Info => "info",
        };
        match (&diag.file, diag.line) {
            (Some(file), Some(line)) => eprintln!("{prefix}: {file}:{line}: {}", diag.message),
            _ => eprintln!("{prefix}: {}", diag.message),
        }
    }

    if verbose {
        if !result.raw_stdout.is_empty() {
            eprintln!("{}", result.raw_stdout);
        }
        if !result.raw_stderr.is_empty() {
            eprintln!("{}", result.raw_stderr);
        }
    }
}
