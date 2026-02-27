//! Compiler invocation and diagnostics normalization.

use std::collections::HashSet;
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
    plugins: Vec<PathBuf>,
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

    /// Add compiler plugin JAR paths (emits `-Xplugin=<path>` for each).
    pub fn plugins(mut self, paths: &[PathBuf]) -> Self {
        self.plugins = paths.to_vec();
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

        // Compiler plugins
        for plugin in &self.plugins {
            args.push(format!("-Xplugin={}", plugin.display()));
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
///
/// Deduplicates by message content so repeated compiler output does not
/// produce duplicate diagnostics.
fn detect_toolchain_errors(stderr: &str, diagnostics: &mut Vec<Diagnostic>) {
    let mut seen: HashSet<String> = diagnostics.iter().map(|d| d.message.clone()).collect();

    let mut push_unique = |diag: Diagnostic| {
        if seen.insert(diag.message.clone()) {
            diagnostics.push(diag);
        }
    };

    // macOS: missing Xcode Command Line Tools
    if stderr.contains("xcode-select")
        || stderr.contains("xcrun")
        || stderr.contains("no developer tools were found")
        || stderr.contains("CommandLineTools")
    {
        push_unique(Diagnostic {
            level: DiagnosticLevel::Error,
            message: "Xcode Command Line Tools not found — run `xcode-select --install`".to_owned(),
            file: None,
            line: None,
        });
    }

    // Linux: missing required system libraries
    if stderr.contains("cannot find -lstdc++") || stderr.contains("cannot find -lm") {
        push_unique(Diagnostic {
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
    fn detect_toolchain_deduplicates_existing_diagnostics() {
        // Pre-populate diagnostics with the same message that detect_toolchain_errors would add
        let mut diags = vec![Diagnostic {
            level: DiagnosticLevel::Error,
            message: "Xcode Command Line Tools not found — run `xcode-select --install`".to_owned(),
            file: None,
            line: None,
        }];
        detect_toolchain_errors("xcode-select: error: something", &mut diags);
        // Should still be 1, not 2
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn detect_toolchain_does_not_duplicate_on_repeated_call() {
        let mut diags = Vec::new();
        let stderr = "ld: cannot find -lstdc++\nld: cannot find -lstdc++";
        detect_toolchain_errors(stderr, &mut diags);
        detect_toolchain_errors(stderr, &mut diags);
        // Should be 1 even after two calls
        assert_eq!(diags.len(), 1);
        assert!(diags.get(0).unwrap().message.contains("build-essential"));
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
    fn build_args_with_plugins() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .plugins(&[
                PathBuf::from("/cache/plugin-a.jar"),
                PathBuf::from("/cache/plugin-b.jar"),
            ]);

        let args = cmd.build_args().unwrap();
        let plugin_args: Vec<_> = args.iter().filter(|a| a.starts_with("-Xplugin=")).collect();
        assert_eq!(plugin_args.len(), 2);
        assert_eq!(
            plugin_args.first().map(|s| s.as_str()),
            Some("-Xplugin=/cache/plugin-a.jar")
        );
        assert_eq!(
            plugin_args.get(1).map(|s| s.as_str()),
            Some("-Xplugin=/cache/plugin-b.jar")
        );
    }

    #[test]
    fn build_args_no_plugins_by_default() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"));

        let args = cmd.build_args().unwrap();
        assert!(
            !args.iter().any(|a| a.starts_with("-Xplugin=")),
            "should not have -Xplugin args by default"
        );
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

    // ── Error variant matching ──────────────────────────────────────────

    #[test]
    fn build_args_no_sources_returns_no_sources_error() {
        let cmd = KonancCommand::new().output(Path::new("out"));
        let err = cmd.build_args().unwrap_err();
        assert!(
            matches!(err, KonancError::NoSources),
            "expected NoSources, got: {err:?}"
        );
    }

    #[test]
    fn build_args_no_output_returns_no_output_error() {
        let cmd = KonancCommand::new().sources(&[PathBuf::from("main.kt")]);
        let err = cmd.build_args().unwrap_err();
        assert!(
            matches!(err, KonancError::NoOutput),
            "expected NoOutput, got: {err:?}"
        );
    }

    #[test]
    fn build_args_empty_sources_returns_no_sources_error() {
        let cmd = KonancCommand::new()
            .sources(&[])
            .output(Path::new("out"));
        let err = cmd.build_args().unwrap_err();
        assert!(matches!(err, KonancError::NoSources));
    }

    // ── Default builder state ───────────────────────────────────────────

    #[test]
    fn default_builder_has_expected_defaults() {
        let cmd = KonancCommand::new();
        assert!(cmd.sources.is_empty());
        assert!(cmd.output.is_none());
        assert!(cmd.target.is_none());
        assert!(!cmd.release);
        assert_eq!(cmd.produce, ProduceKind::Program);
        assert!(cmd.libraries.is_empty());
        assert!(cmd.plugins.is_empty());
        assert!(cmd.java_home.is_none());
        assert!(!cmd.generate_test_runner);
    }

    #[test]
    fn produce_kind_default_is_program() {
        assert_eq!(ProduceKind::default(), ProduceKind::Program);
    }

    // ── Argument ordering ───────────────────────────────────────────────

    #[test]
    fn build_args_full_ordering() {
        // Verify the exact ordering: sources, -o, output, -target, target,
        // -produce, library, -library, libs, -Xplugin, -generate-test-runner, -opt
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("a.kt"), PathBuf::from("b.kt")])
            .output(Path::new("out/bin"))
            .target("macos_arm64")
            .produce(ProduceKind::Library)
            .libraries(&[PathBuf::from("dep.klib")])
            .plugins(&[PathBuf::from("plugin.jar")])
            .generate_test_runner(true)
            .release(true);

        let args = cmd.build_args().unwrap();
        assert_eq!(
            args,
            vec![
                "a.kt",
                "b.kt",
                "-o",
                "out/bin",
                "-target",
                "macos_arm64",
                "-produce",
                "library",
                "-library",
                "dep.klib",
                "-Xplugin=plugin.jar",
                "-generate-test-runner",
                "-opt",
            ]
        );
    }

    #[test]
    fn build_args_sources_come_before_output() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("x.kt")])
            .output(Path::new("out"));

        let args = cmd.build_args().unwrap();
        let src_pos = args.iter().position(|a| a == "x.kt").unwrap();
        let out_pos = args.iter().position(|a| a == "-o").unwrap();
        assert!(
            src_pos < out_pos,
            "source files must come before -o flag"
        );
    }

    #[test]
    fn build_args_opt_is_last() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .target("linux_x64")
            .release(true);

        let args = cmd.build_args().unwrap();
        assert_eq!(
            args.last().map(|s| s.as_str()),
            Some("-opt"),
            "-opt should be the last argument"
        );
    }

    // ── Empty libraries/plugins produce no flags ────────────────────────

    #[test]
    fn build_args_empty_libraries_produces_no_library_flags() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .libraries(&[]);

        let args = cmd.build_args().unwrap();
        assert!(
            !args.contains(&"-library".to_owned()),
            "empty libraries should not produce -library flags"
        );
    }

    #[test]
    fn build_args_empty_plugins_produces_no_plugin_flags() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .plugins(&[]);

        let args = cmd.build_args().unwrap();
        assert!(
            !args.iter().any(|a| a.starts_with("-Xplugin=")),
            "empty plugins should not produce -Xplugin flags"
        );
    }

    // ── No target omits -target flag ────────────────────────────────────

    #[test]
    fn build_args_no_target_omits_target_flag() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"));

        let args = cmd.build_args().unwrap();
        assert!(
            !args.contains(&"-target".to_owned()),
            "no target set should omit -target flag"
        );
    }

    // ── Diagnostic parsing: info level ──────────────────────────────────

    #[test]
    fn parse_diagnostics_bare_info() {
        let diags = parse_diagnostics("info: some informational message\n");
        assert_eq!(diags.len(), 1);
        let d = diags.get(0).unwrap();
        assert_eq!(d.level, DiagnosticLevel::Info);
        assert_eq!(d.message, "some informational message");
        assert!(d.file.is_none());
        assert!(d.line.is_none());
    }

    #[test]
    fn parse_diagnostics_located_info() {
        let diags = parse_diagnostics("utils.kt:3:1: info: additional context");
        assert_eq!(diags.len(), 1);
        let d = diags.get(0).unwrap();
        assert_eq!(d.level, DiagnosticLevel::Info);
        assert_eq!(d.file, Some("utils.kt".to_owned()));
        assert_eq!(d.line, Some(3));
        assert_eq!(d.message, "additional context");
    }

    // ── Diagnostic parsing: whitespace and edge cases ───────────────────

    #[test]
    fn parse_diagnostics_whitespace_only_is_empty() {
        let diags = parse_diagnostics("   \n  \n\n  \t \n");
        assert!(diags.is_empty());
    }

    #[test]
    fn parse_diagnostics_leading_trailing_whitespace_trimmed() {
        let diags = parse_diagnostics("  error: something bad  \n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.get(0).unwrap().message, "something bad");
    }

    #[test]
    fn parse_diagnostics_mixed_levels() {
        let stderr = "error: first\nwarning: second\ninfo: third\n";
        let diags = parse_diagnostics(stderr);
        assert_eq!(diags.len(), 3);
        assert_eq!(diags.get(0).unwrap().level, DiagnosticLevel::Error);
        assert_eq!(diags.get(1).unwrap().level, DiagnosticLevel::Warning);
        assert_eq!(diags.get(2).unwrap().level, DiagnosticLevel::Info);
    }

    #[test]
    fn parse_diagnostics_interleaved_with_noise() {
        let stderr = "\
compiler: initializing
src/main.kt:1:1: error: unresolved reference 'foo'
some other output
warning: deprecated API usage
more noise
";
        let diags = parse_diagnostics(stderr);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags.get(0).unwrap().level, DiagnosticLevel::Error);
        assert!(diags.get(0).unwrap().file.is_some());
        assert_eq!(diags.get(1).unwrap().level, DiagnosticLevel::Warning);
        assert!(diags.get(1).unwrap().file.is_none());
    }

    // ── Diagnostic parsing: located with deep paths ─────────────────────

    #[test]
    fn parse_diagnostics_located_deep_path() {
        let diags =
            parse_diagnostics("src/main/kotlin/App.kt:42:10: error: type mismatch");
        assert_eq!(diags.len(), 1);
        let d = diags.get(0).unwrap();
        assert_eq!(d.file, Some("src/main/kotlin/App.kt".to_owned()));
        assert_eq!(d.line, Some(42));
        assert_eq!(d.message, "type mismatch");
    }

    // ── CompilationResult: error_count and warning_count ────────────────

    #[test]
    fn compilation_result_error_count_only_counts_errors() {
        let result = CompilationResult {
            success: false,
            output_path: PathBuf::from("out"),
            diagnostics: vec![
                Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: "e1".to_owned(),
                    file: None,
                    line: None,
                },
                Diagnostic {
                    level: DiagnosticLevel::Warning,
                    message: "w1".to_owned(),
                    file: None,
                    line: None,
                },
                Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: "e2".to_owned(),
                    file: None,
                    line: None,
                },
                Diagnostic {
                    level: DiagnosticLevel::Info,
                    message: "i1".to_owned(),
                    file: None,
                    line: None,
                },
            ],
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        };
        assert_eq!(result.error_count(), 2);
    }

    #[test]
    fn compilation_result_warning_count_only_counts_warnings() {
        let result = CompilationResult {
            success: true,
            output_path: PathBuf::from("out"),
            diagnostics: vec![
                Diagnostic {
                    level: DiagnosticLevel::Warning,
                    message: "w1".to_owned(),
                    file: None,
                    line: None,
                },
                Diagnostic {
                    level: DiagnosticLevel::Error,
                    message: "e1".to_owned(),
                    file: None,
                    line: None,
                },
                Diagnostic {
                    level: DiagnosticLevel::Warning,
                    message: "w2".to_owned(),
                    file: None,
                    line: None,
                },
                Diagnostic {
                    level: DiagnosticLevel::Warning,
                    message: "w3".to_owned(),
                    file: None,
                    line: None,
                },
            ],
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        };
        assert_eq!(result.warning_count(), 3);
    }

    #[test]
    fn compilation_result_zero_counts_for_empty_diagnostics() {
        let result = CompilationResult {
            success: true,
            output_path: PathBuf::from("out"),
            diagnostics: vec![],
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        };
        assert_eq!(result.error_count(), 0);
        assert_eq!(result.warning_count(), 0);
    }

    // ── detect_toolchain_errors: additional trigger variants ────────────

    #[test]
    fn detect_toolchain_xcrun_trigger() {
        let mut diags = Vec::new();
        detect_toolchain_errors("xcrun: error: unable to find utility", &mut diags);
        assert_eq!(diags.len(), 1);
        assert!(diags.get(0).unwrap().message.contains("xcode-select --install"));
    }

    #[test]
    fn detect_toolchain_command_line_tools_trigger() {
        let mut diags = Vec::new();
        detect_toolchain_errors(
            "error: unable to find CommandLineTools installation",
            &mut diags,
        );
        assert_eq!(diags.len(), 1);
        assert!(diags.get(0).unwrap().message.contains("xcode-select --install"));
    }

    #[test]
    fn detect_toolchain_cannot_find_lm_trigger() {
        let mut diags = Vec::new();
        detect_toolchain_errors("ld: cannot find -lm", &mut diags);
        assert_eq!(diags.len(), 1);
        assert!(diags.get(0).unwrap().message.contains("build-essential"));
    }

    #[test]
    fn detect_toolchain_both_xcode_and_linux_errors() {
        let mut diags = Vec::new();
        let stderr = "xcode-select: missing tools\ncannot find -lstdc++";
        detect_toolchain_errors(stderr, &mut diags);
        // Should produce two distinct diagnostics
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().any(|d| d.message.contains("xcode-select --install")));
        assert!(diags.iter().any(|d| d.message.contains("build-essential")));
    }

    #[test]
    fn detect_toolchain_errors_are_always_error_level() {
        let mut diags = Vec::new();
        detect_toolchain_errors("xcode-select: missing", &mut diags);
        detect_toolchain_errors("cannot find -lstdc++", &mut diags);
        for d in &diags {
            assert_eq!(d.level, DiagnosticLevel::Error);
        }
    }

    #[test]
    fn detect_toolchain_errors_have_no_file_or_line() {
        let mut diags = Vec::new();
        detect_toolchain_errors("xcode-select: error: something", &mut diags);
        let d = diags.get(0).unwrap();
        assert!(d.file.is_none());
        assert!(d.line.is_none());
    }

    // ── Builder overwrites previous values ──────────────────────────────

    #[test]
    fn builder_sources_replaces_previous() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("old.kt")])
            .sources(&[PathBuf::from("new.kt")]);

        let args = cmd.build_args().unwrap_err(); // no output set, but sources replaced
        // Verify we cannot build without output, meaning sources was indeed set
        assert!(matches!(args, KonancError::NoOutput));
    }

    #[test]
    fn builder_output_replaces_previous() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("first"))
            .output(Path::new("second"));

        let args = cmd.build_args().unwrap();
        assert!(args.contains(&"second".to_owned()));
        assert!(!args.contains(&"first".to_owned()));
    }

    #[test]
    fn builder_target_replaces_previous() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .target("linux_x64")
            .target("macos_arm64");

        let args = cmd.build_args().unwrap();
        assert!(args.contains(&"macos_arm64".to_owned()));
        assert!(!args.contains(&"linux_x64".to_owned()));
    }

    #[test]
    fn builder_libraries_replaces_previous() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .libraries(&[PathBuf::from("old.klib")])
            .libraries(&[PathBuf::from("new.klib")]);

        let args = cmd.build_args().unwrap();
        let lib_values: Vec<_> = args
            .windows(2)
            .filter(|w| w[0] == "-library")
            .map(|w| w[1].clone())
            .collect();
        assert_eq!(lib_values, vec!["new.klib"]);
    }

    #[test]
    fn builder_plugins_replaces_previous() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"))
            .plugins(&[PathBuf::from("old.jar")])
            .plugins(&[PathBuf::from("new.jar")]);

        let args = cmd.build_args().unwrap();
        let plugin_args: Vec<_> = args.iter().filter(|a| a.starts_with("-Xplugin=")).collect();
        assert_eq!(plugin_args.len(), 1);
        assert_eq!(plugin_args[0], "-Xplugin=new.jar");
    }

    // ── Single source file ──────────────────────────────────────────────

    #[test]
    fn build_args_single_source_is_first_arg() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"));

        let args = cmd.build_args().unwrap();
        assert_eq!(args.get(0).unwrap(), "main.kt");
    }

    // ── Output path with directory components ───────────────────────────

    #[test]
    fn build_args_output_preserves_path() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new(".konvoy/build/linux_x64/debug/myapp"));

        let args = cmd.build_args().unwrap();
        let o_idx = args.iter().position(|a| a == "-o").unwrap();
        assert_eq!(
            args.get(o_idx + 1).unwrap(),
            ".konvoy/build/linux_x64/debug/myapp"
        );
    }

    // ── Compilation summary edge case: failure with no errors ───────────

    #[test]
    fn compilation_result_summary_failure_with_zero_errors() {
        // Edge case: process exited non-zero but no parseable error diagnostics
        let result = CompilationResult {
            success: false,
            output_path: PathBuf::from("out"),
            diagnostics: vec![],
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        };
        assert_eq!(result.summary(), "compilation failed with 0 error(s)");
    }

    // ── parse_diagnostics: message content edge cases ───────────────────

    #[test]
    fn parse_diagnostics_error_with_colon_in_message() {
        // The message itself contains ":" — should not confuse the parser
        let diags = parse_diagnostics("error: type mismatch: expected Int, found String");
        assert_eq!(diags.len(), 1);
        assert_eq!(
            diags.get(0).unwrap().message,
            "type mismatch: expected Int, found String"
        );
    }

    #[test]
    fn parse_diagnostics_located_error_message_with_colons() {
        let diags = parse_diagnostics(
            "src/main.kt:1:1: error: unresolved reference: myFunc",
        );
        assert_eq!(diags.len(), 1);
        let d = diags.get(0).unwrap();
        assert_eq!(d.file, Some("src/main.kt".to_owned()));
        assert_eq!(d.line, Some(1));
        assert_eq!(d.message, "unresolved reference: myFunc");
    }

    #[test]
    fn parse_diagnostics_warning_with_empty_message() {
        let diags = parse_diagnostics("warning:");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.get(0).unwrap().level, DiagnosticLevel::Warning);
        assert_eq!(diags.get(0).unwrap().message, "");
    }

    // ── Library produce kind includes -produce library pair ─────────────

    #[test]
    fn build_args_library_produce_flag_pair_is_adjacent() {
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("lib.kt")])
            .output(Path::new("out.klib"))
            .produce(ProduceKind::Library);

        let args = cmd.build_args().unwrap();
        let produce_idx = args.iter().position(|a| a == "-produce").unwrap();
        assert_eq!(args.get(produce_idx + 1).unwrap(), "library");
    }

    // ── Minimal valid command ───────────────────────────────────────────

    #[test]
    fn build_args_minimal_valid_command() {
        // Only sources and output are required
        let cmd = KonancCommand::new()
            .sources(&[PathBuf::from("main.kt")])
            .output(Path::new("out"));

        let args = cmd.build_args().unwrap();
        assert_eq!(args, vec!["main.kt", "-o", "out"]);
    }
}
