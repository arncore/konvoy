//! Compiler invocation and diagnostics normalization.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::detect::KonancInfo;
use crate::error::KonancError;

/// Severity level of a compiler diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Info,
}

/// A single structured diagnostic from the compiler.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Severity level.
    pub level: DiagnosticLevel,
    /// Human-readable message.
    pub message: String,
    /// Source file path, if available.
    pub file: Option<String>,
    /// Line number in the source file, if available.
    pub line: Option<u32>,
}

/// Result of a compilation invocation.
#[derive(Debug)]
pub struct CompilationResult {
    /// Whether compilation succeeded.
    pub success: bool,
    /// Path to the output binary (may not exist if compilation failed).
    pub output_path: PathBuf,
    /// Parsed diagnostics from compiler output.
    pub diagnostics: Vec<Diagnostic>,
    /// Raw stdout from the compiler.
    pub raw_stdout: String,
    /// Raw stderr from the compiler.
    pub raw_stderr: String,
}

impl CompilationResult {
    /// Count the number of error-level diagnostics.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Error)
            .count()
    }

    /// Count the number of warning-level diagnostics.
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Warning)
            .count()
    }

    /// Format a human-readable summary of the compilation result.
    pub fn summary(&self) -> String {
        if self.success {
            let warnings = self.warning_count();
            if warnings > 0 {
                format!("compilation succeeded with {warnings} warning(s)")
            } else {
                "compilation succeeded".to_owned()
            }
        } else {
            let errors = self.error_count();
            format!("compilation failed with {errors} error(s)")
        }
    }
}

/// What kind of output to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProduceKind {
    /// A native executable (default).
    #[default]
    Program,
    /// A Kotlin/Native library (`.klib`).
    Library,
}

/// Builder for constructing a `konanc` invocation.
#[derive(Debug, Default)]
pub struct KonancCommand {
    sources: Vec<PathBuf>,
    output: Option<PathBuf>,
    target: Option<String>,
    release: bool,
    produce: ProduceKind,
    libraries: Vec<PathBuf>,
    java_home: Option<PathBuf>,
    generate_test_runner: bool,
}

impl KonancCommand {
    /// Create a new empty command builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the source files to compile.
    pub fn sources(mut self, paths: &[PathBuf]) -> Self {
        self.sources = paths.to_vec();
        self
    }

    /// Set the output binary path.
    pub fn output(mut self, path: &Path) -> Self {
        self.output = Some(path.to_path_buf());
        self
    }

    /// Set the Kotlin/Native target (e.g. "linux_x64").
    pub fn target(mut self, target: &str) -> Self {
        self.target = Some(target.to_owned());
        self
    }

    /// Enable release mode (adds `-opt` flag).
    pub fn release(mut self, enabled: bool) -> Self {
        self.release = enabled;
        self
    }

    /// Set the output kind (program or library).
    pub fn produce(mut self, kind: ProduceKind) -> Self {
        self.produce = kind;
        self
    }

    /// Add dependency library paths (`.klib` files).
    pub fn libraries(mut self, paths: &[PathBuf]) -> Self {
        self.libraries = paths.to_vec();
        self
    }

    /// Set JAVA_HOME for the bundled JRE.
    pub fn java_home(mut self, path: &Path) -> Self {
        self.java_home = Some(path.to_path_buf());
        self
    }

    /// Enable test runner generation (adds `-generate-test-runner` flag).
    pub fn generate_test_runner(mut self, enabled: bool) -> Self {
        self.generate_test_runner = enabled;
        self
    }

    /// Build the argument list without executing.
    ///
    /// # Errors
    /// Returns an error if sources or output path are not set.
    pub fn build_args(&self) -> Result<Vec<String>, KonancError> {
        if self.sources.is_empty() {
            return Err(KonancError::NoSources);
        }
        let Some(output) = &self.output else {
            return Err(KonancError::NoOutput);
        };

        let mut args = Vec::new();

        // Source files first
        for src in &self.sources {
            args.push(src.display().to_string());
        }

        // Output path
        args.push("-o".to_owned());
        args.push(output.display().to_string());

        // Target
        if let Some(target) = &self.target {
            args.push("-target".to_owned());
            args.push(target.clone());
        }

        // Produce kind
        match self.produce {
            ProduceKind::Program => {} // default, no flag needed
            ProduceKind::Library => {
                args.push("-produce".to_owned());
                args.push("library".to_owned());
            }
        }

        // Dependency libraries
        for lib in &self.libraries {
            args.push("-library".to_owned());
            args.push(lib.display().to_string());
        }

        // Test runner generation
        if self.generate_test_runner {
            args.push("-generate-test-runner".to_owned());
        }

        // Release optimization
        if self.release {
            args.push("-opt".to_owned());
        }

        Ok(args)
    }

    /// Execute the compilation using the given konanc installation.
    ///
    /// # Errors
    /// Returns an error if:
    /// - Sources or output path are not set
    /// - The konanc binary cannot be executed
    pub fn execute(&self, konanc: &KonancInfo) -> Result<CompilationResult, KonancError> {
        let args = self.build_args()?;
        let Some(output_path) = &self.output else {
            return Err(KonancError::NoOutput);
        };

        let mut cmd = Command::new(&konanc.path);
        cmd.args(&args);
        if let Some(jh) = &self.java_home {
            cmd.env("JAVA_HOME", jh);
        }
        let cmd_output = cmd
            .output()
            .map_err(|source| KonancError::Exec { source })?;

        let raw_stdout = String::from_utf8_lossy(&cmd_output.stdout).into_owned();
        let raw_stderr = String::from_utf8_lossy(&cmd_output.stderr).into_owned();

        let mut diagnostics = parse_diagnostics(&raw_stderr);
        detect_toolchain_errors(&raw_stderr, &mut diagnostics);

        Ok(CompilationResult {
            success: cmd_output.status.success(),
            output_path: output_path.clone(),
            diagnostics,
            raw_stdout,
            raw_stderr,
        })
    }
}

/// Parse compiler stderr into structured diagnostics.
///
/// Handles common konanc output formats:
/// - `file.kt:10:5: error: message`
/// - `error: message`
/// - `warning: message`
pub fn parse_diagnostics(stderr: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(diag) = try_parse_located_diagnostic(trimmed) {
            diagnostics.push(diag);
        } else if let Some(diag) = try_parse_bare_diagnostic(trimmed) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

/// Try to parse a diagnostic with file location: `file.kt:10:5: error: message`
fn try_parse_located_diagnostic(line: &str) -> Option<Diagnostic> {
    // Pattern: <file>:<line>:<col>: <level>: <message>
    // or:      <file>:<line>: <level>: <message>
    let (file_part, rest) = split_file_location(line)?;

    let (level, message) = parse_level_message(rest)?;

    Some(Diagnostic {
        level,
        message,
        file: Some(file_part.file),
        line: Some(file_part.line),
    })
}

struct FileLocation {
    file: String,
    line: u32,
}

fn split_file_location(line: &str) -> Option<(FileLocation, &str)> {
    // Find the pattern: something.ext:digits: or something.ext:digits:digits:
    // We look for ": error:" or ": warning:" or ": info:" after the location
    for level_prefix in &[": error:", ": warning:", ": info:"] {
        if let Some(pos) = line.find(level_prefix) {
            let before = line.get(..pos)?;
            let after = line.get(pos + 2..)?; // skip ": "

            // before should be like "file.kt:10:5" or "file.kt:10"
            if let Some(loc) = parse_file_and_line(before) {
                return Some((loc, after));
            }
        }
    }
    None
}

fn parse_file_and_line(s: &str) -> Option<FileLocation> {
    // Try "file:line:col" first, then "file:line"
    let mut parts: Vec<&str> = s.rsplitn(3, ':').collect();
    parts.reverse();

    match parts.len() {
        3 => {
            // file:line:col
            let file = (*parts.first()?).to_owned();
            let line: u32 = parts.get(1)?.parse().ok()?;
            Some(FileLocation { file, line })
        }
        2 => {
            // file:line
            let file = (*parts.first()?).to_owned();
            let line: u32 = parts.get(1)?.parse().ok()?;
            Some(FileLocation { file, line })
        }
        _ => None,
    }
}

/// Try to parse a bare diagnostic: `error: message` or `warning: message`
fn try_parse_bare_diagnostic(line: &str) -> Option<Diagnostic> {
    let (level, message) = parse_level_message(line)?;
    Some(Diagnostic {
        level,
        message,
        file: None,
        line: None,
    })
}

fn parse_level_message(s: &str) -> Option<(DiagnosticLevel, String)> {
    let prefixes = [
        ("error:", DiagnosticLevel::Error),
        ("warning:", DiagnosticLevel::Warning),
        ("info:", DiagnosticLevel::Info),
    ];

    prefixes.into_iter().find_map(|(prefix, level)| {
        s.strip_prefix(prefix)
            .map(|msg| (level, msg.trim().to_owned()))
    })
}

/// Detect platform toolchain errors and add actionable diagnostics.
fn detect_toolchain_errors(stderr: &str, diagnostics: &mut Vec<Diagnostic>) {
    // macOS: missing Xcode Command Line Tools
    if stderr.contains("xcode-select")
        || stderr.contains("xcrun")
        || stderr.contains("no developer tools were found")
        || stderr.contains("CommandLineTools")
    {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            message: "Xcode Command Line Tools not found — run `xcode-select --install`".to_owned(),
            file: None,
            line: None,
        });
    }

    // Linux: missing required system libraries
    if stderr.contains("cannot find -lstdc++") || stderr.contains("cannot find -lm") {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            message:
                "missing system libraries — install build-essential: `sudo apt install build-essential`"
                    .to_owned(),
            file: None,
            line: None,
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn build_args_basic() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("src/main.kt")])
            .output(Path::new("build/app"))
            .target("linux_x64");

        let args = cmd.build_args().unwrap();
        assert_eq!(
            args,
            vec!["src/main.kt", "-o", "build/app", "-target", "linux_x64"]
        );
    }

    #[test]
    fn build_args_release() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .release(true);

        let args = cmd.build_args().unwrap();
        assert!(args.contains(&"-opt".to_owned()));
    }

    #[test]
    fn build_args_no_release() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .release(false);

        let args = cmd.build_args().unwrap();
        assert!(!args.contains(&"-opt".to_owned()));
    }

    #[test]
    fn build_args_multiple_sources() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("a.kt"), PathBuf::from("b.kt")])
            .output(Path::new("out"));

        let args = cmd.build_args().unwrap();
        assert_eq!(args.get(0), Some(&"a.kt".to_owned()));
        assert_eq!(args.get(1), Some(&"b.kt".to_owned()));
        assert_eq!(args.get(2), Some(&"-o".to_owned()));
    }

    #[test]
    fn build_args_no_sources_errors() {
        let cmd = KonancCommand::new().output(Path::new("out"));
        assert!(cmd.build_args().is_err());
    }

    #[test]
    fn build_args_no_output_errors() {
        let cmd = KonancCommand::new().sources(&[PathBuf::from("main.kt")]);
        assert!(cmd.build_args().is_err());
    }

    #[test]
    fn parse_diagnostics_bare_error() {
        let diags = parse_diagnostics("error: unresolved reference: foo\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.get(0).unwrap().level, DiagnosticLevel::Error);
        assert_eq!(diags.get(0).unwrap().message, "unresolved reference: foo");
        assert!(diags.get(0).unwrap().file.is_none());
    }

    #[test]
    fn parse_diagnostics_bare_warning() {
        let diags = parse_diagnostics("warning: parameter 'x' is never used\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.get(0).unwrap().level, DiagnosticLevel::Warning);
    }

    #[test]
    fn parse_diagnostics_located_error() {
        let diags = parse_diagnostics("src/main.kt:10:5: error: expecting ')'");
        assert_eq!(diags.len(), 1);
        let d = diags.get(0).unwrap();
        assert_eq!(d.level, DiagnosticLevel::Error);
        assert_eq!(d.file, Some("src/main.kt".to_owned()));
        assert_eq!(d.line, Some(10));
        assert_eq!(d.message, "expecting ')'");
    }

    #[test]
    fn parse_diagnostics_located_without_column() {
        let diags = parse_diagnostics("main.kt:5: warning: unused variable 'x'");
        assert_eq!(diags.len(), 1);
        let d = diags.get(0).unwrap();
        assert_eq!(d.level, DiagnosticLevel::Warning);
        assert_eq!(d.file, Some("main.kt".to_owned()));
        assert_eq!(d.line, Some(5));
    }

    #[test]
    fn parse_diagnostics_multiple() {
        let stderr = "error: first\nwarning: second\nerror: third\n";
        let diags = parse_diagnostics(stderr);
        assert_eq!(diags.len(), 3);
    }

    #[test]
    fn parse_diagnostics_empty() {
        let diags = parse_diagnostics("");
        assert!(diags.is_empty());
    }

    #[test]
    fn parse_diagnostics_non_diagnostic_lines_skipped() {
        let stderr = "some info line\nerror: real problem\nanother line\n";
        let diags = parse_diagnostics(stderr);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.get(0).unwrap().message, "real problem");
    }

    #[test]
    fn detect_toolchain_xcode_missing() {
        let mut diags = Vec::new();
        detect_toolchain_errors(
            "xcode-select: error: no developer tools were found",
            &mut diags,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags
            .iter()
            .any(|d| d.message.contains("xcode-select --install")));
    }

    #[test]
    fn detect_toolchain_linux_missing_libs() {
        let mut diags = Vec::new();
        detect_toolchain_errors("ld: cannot find -lstdc++", &mut diags);
        assert_eq!(diags.len(), 1);
        assert!(diags.get(0).unwrap().message.contains("build-essential"));
    }

    #[test]
    fn detect_toolchain_no_issues() {
        let mut diags = Vec::new();
        detect_toolchain_errors("normal compiler output", &mut diags);
        assert!(diags.is_empty());
    }

    #[test]
    fn compilation_result_summary_success() {
        let result = CompilationResult {
            success: true,
            output_path: PathBuf::from("out"),
            diagnostics: vec![],
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        };
        assert_eq!(result.summary(), "compilation succeeded");
    }

    #[test]
    fn compilation_result_summary_with_warnings() {
        let result = CompilationResult {
            success: true,
            output_path: PathBuf::from("out"),
            diagnostics: vec![Diagnostic {
                level: DiagnosticLevel::Warning,
                message: "unused".to_owned(),
                file: None,
                line: None,
            }],
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        };
        assert_eq!(result.summary(), "compilation succeeded with 1 warning(s)");
    }

    #[test]
    fn compilation_result_summary_failure() {
        let result = CompilationResult {
            success: false,
            output_path: PathBuf::from("out"),
            diagnostics: vec![
                Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: "err1".to_owned(),
                    file: None,
                    line: None,
                },
                Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: "err2".to_owned(),
                    file: None,
                    line: None,
                },
            ],
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        };
        assert_eq!(result.summary(), "compilation failed with 2 error(s)");
    }

    #[test]
    fn konanc_command_builder_is_fluent() {
        // Ensure builder pattern works chained
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("a.kt")])
            .output(Path::new("out"))
            .target("linux_x64")
            .release(true);

        let args = cmd.build_args().unwrap();
        assert_eq!(args.len(), 6); // a.kt -o out -target linux_x64 -opt
    }

    #[test]
    fn build_args_produce_library() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("lib.kt")])
            .output(Path::new("out.klib"))
            .produce(ProduceKind::Library);

        let args = cmd.build_args().unwrap();
        assert!(args.contains(&"-produce".to_owned()));
        assert!(args.contains(&"library".to_owned()));
    }

    #[test]
    fn build_args_produce_program_omits_flag() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .produce(ProduceKind::Program);

        let args = cmd.build_args().unwrap();
        assert!(!args.contains(&"-produce".to_owned()));
    }

    #[test]
    fn java_home_not_set_by_default() {
        let cmd = KonancCommand::new();
        assert!(cmd.java_home.is_none());
    }

    #[test]
    fn java_home_builder_sets_value() {
        let cmd = KonancCommand::new().java_home(Path::new("/opt/jre"));
        assert_eq!(cmd.java_home, Some(PathBuf::from("/opt/jre")));
    }

    #[test]
    fn build_args_generate_test_runner_enabled() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("test.kt")])
            .output(Path::new("test_out"))
            .generate_test_runner(true);

        let args = cmd.build_args().unwrap();
        assert!(args.contains(&"-generate-test-runner".to_owned()));
    }

    #[test]
    fn build_args_generate_test_runner_disabled() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("test.kt")])
            .output(Path::new("test_out"))
            .generate_test_runner(false);

        let args = cmd.build_args().unwrap();
        assert!(!args.contains(&"-generate-test-runner".to_owned()));
    }

    #[test]
    fn build_args_generate_test_runner_default_off() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("test.kt")])
            .output(Path::new("test_out"));

        let args = cmd.build_args().unwrap();
        assert!(!args.contains(&"-generate-test-runner".to_owned()));
    }

    #[test]
    fn build_args_with_libraries() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .libraries(&[PathBuf::from("dep.klib"), PathBuf::from("other.klib")]);

        let args = cmd.build_args().unwrap();
        let lib_indices: Vec<_> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| a.as_str() == "-library")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(lib_indices.len(), 2);
        assert_eq!(args.get(lib_indices[0] + 1).unwrap(), "dep.klib");
        assert_eq!(args.get(lib_indices[1] + 1).unwrap(), "other.klib");
    }
}
