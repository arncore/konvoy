#![forbid(unsafe_code)]

use std::error::Error;
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

type CliResult = Result<(), Box<dyn Error>>;

#[derive(Debug, Parser)]
#[command(name = "konvoy", about = "A native-first Kotlin build tool")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new Konvoy project
    Init {
        /// Project name
        #[arg(long)]
        name: Option<String>,
        /// Create a library project instead of a binary
        #[arg(long)]
        lib: bool,
    },
    /// Compile the project
    Build {
        /// Target triple (defaults to host)
        #[arg(long)]
        target: Option<String>,
        /// Build in release mode
        #[arg(long)]
        release: bool,
        /// Show compiler output
        #[arg(long)]
        verbose: bool,
        /// Force a rebuild, bypassing the cache
        #[arg(long)]
        force: bool,
        /// Require the lockfile to be up-to-date; error on any mismatch
        #[arg(long)]
        locked: bool,
    },
    /// Build and run the project
    Run {
        /// Target triple (defaults to host)
        #[arg(long)]
        target: Option<String>,
        /// Run in release mode
        #[arg(long)]
        release: bool,
        /// Show compiler output
        #[arg(long, short = 'v')]
        verbose: bool,
        /// Force a rebuild, bypassing the cache
        #[arg(long)]
        force: bool,
        /// Require the lockfile to be up-to-date; error on any mismatch
        #[arg(long)]
        locked: bool,
        /// Arguments to pass to the program
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run tests
    Test {
        /// Target triple (defaults to host)
        #[arg(long)]
        target: Option<String>,
        /// Test in release mode
        #[arg(long)]
        release: bool,
        /// Show compiler output
        #[arg(long)]
        verbose: bool,
        /// Force a rebuild, bypassing the cache
        #[arg(long)]
        force: bool,
        /// Require the lockfile to be up-to-date; error on any mismatch
        #[arg(long)]
        locked: bool,
        /// Only run tests matching this pattern (forwarded to --ktest_filter)
        #[arg(long)]
        filter: Option<String>,
    },
    /// Run detekt linter on Kotlin source files
    Lint {
        /// Show raw detekt output
        #[arg(long)]
        verbose: bool,
        /// Path to a custom detekt configuration file
        #[arg(long)]
        config: Option<PathBuf>,
        /// Require the lockfile to be up-to-date; error on any mismatch
        #[arg(long)]
        locked: bool,
    },
    /// Remove build artifacts
    Clean,
    /// Check environment and toolchain setup
    Doctor,
    /// Manage Kotlin/Native toolchains
    Toolchain {
        #[command(subcommand)]
        action: ToolchainAction,
    },
}

#[derive(Debug, Subcommand)]
enum ToolchainAction {
    /// Install a Kotlin/Native version
    Install {
        /// Kotlin/Native version (e.g. "2.1.0"). If omitted, reads from konvoy.toml.
        version: Option<String>,
    },
    /// List installed Kotlin/Native versions
    List,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Init { name, lib } => cmd_init(name, lib),
        Command::Build {
            target,
            release,
            verbose,
            force,
            locked,
        } => cmd_build(target, release, verbose, force, locked),
        Command::Run {
            target,
            release,
            verbose,
            force,
            locked,
            args,
        } => cmd_run(target, release, verbose, force, locked, &args),
        Command::Test {
            target,
            release,
            verbose,
            force,
            locked,
            filter,
        } => cmd_test(target, release, verbose, force, locked, &filter),
        Command::Lint {
            verbose,
            config,
            locked,
        } => cmd_lint(verbose, config, locked),
        Command::Clean => cmd_clean(),
        Command::Doctor => cmd_doctor(),
        Command::Toolchain { action } => cmd_toolchain(action),
    };

    if let Err(msg) = result {
        eprintln!("error: {msg}");
        process::exit(1);
    }
}

/// Find the project root by looking for `konvoy.toml` in the current directory.
fn project_root() -> Result<PathBuf, Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let manifest = cwd.join("konvoy.toml");
    if !manifest.exists() {
        return Err(
            "no konvoy.toml found in current directory — run `konvoy init` to create a project"
                .into(),
        );
    }
    Ok(cwd)
}

fn cmd_init(name: Option<String>, lib: bool) -> CliResult {
    let cwd = std::env::current_dir()?;

    let project_name = name.unwrap_or_else(|| {
        cwd.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("my-project")
            .to_owned()
    });

    let project_dir = cwd.join(&project_name);

    let kind = if lib {
        konvoy_config::manifest::PackageKind::Lib
    } else {
        konvoy_config::manifest::PackageKind::Bin
    };

    konvoy_engine::init_project_with_kind(&project_name, &project_dir, kind)?;

    let kind_label = if lib { "library" } else { "project" };
    eprintln!(
        "    Created {kind_label} `{project_name}` at {}",
        project_dir.display()
    );
    eprintln!();
    eprintln!("  To get started:");
    eprintln!("    cd {project_name}");
    eprintln!("    konvoy build");
    Ok(())
}

fn cmd_build(
    target: Option<String>,
    release: bool,
    verbose: bool,
    force: bool,
    locked: bool,
) -> CliResult {
    let root = project_root()?;
    let options = konvoy_engine::BuildOptions {
        target,
        release,
        verbose,
        force,
        locked,
    };

    let result = konvoy_engine::build(&root, &options)?;

    let profile = if release { "release" } else { "debug" };
    match result.outcome {
        konvoy_engine::BuildOutcome::Cached => {
            eprintln!(
                "    Finished `{profile}` target in {:.2}s (cached)",
                result.duration.as_secs_f64()
            );
        }
        konvoy_engine::BuildOutcome::Fresh => {
            eprintln!(
                "    Finished `{profile}` target in {:.2}s",
                result.duration.as_secs_f64()
            );
        }
    }

    Ok(())
}

fn cmd_run(
    target: Option<String>,
    release: bool,
    verbose: bool,
    force: bool,
    locked: bool,
    args: &[String],
) -> CliResult {
    let root = project_root()?;

    // Cannot run a library project.
    let manifest = konvoy_config::Manifest::from_path(&root.join("konvoy.toml"))?;
    if manifest.package.kind == konvoy_config::manifest::PackageKind::Lib {
        return Err(
            "cannot run a library project — only binary projects (kind = \"bin\") can be run"
                .into(),
        );
    }

    let options = konvoy_engine::BuildOptions {
        target,
        release,
        verbose,
        force,
        locked,
    };

    let result = konvoy_engine::build(&root, &options)?;

    let profile = if release { "release" } else { "debug" };
    eprintln!(
        "    Finished `{profile}` target in {:.2}s",
        result.duration.as_secs_f64()
    );
    eprintln!("     Running `{}`", result.output_path.display());

    let status = std::process::Command::new(&result.output_path)
        .args(args)
        .status()
        .map_err(|e| format!("cannot run {}: {e}", result.output_path.display()))?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        process::exit(code);
    }

    Ok(())
}

fn cmd_test(
    target: Option<String>,
    release: bool,
    verbose: bool,
    force: bool,
    locked: bool,
    filter: &Option<String>,
) -> CliResult {
    let root = project_root()?;
    let options = konvoy_engine::TestOptions {
        target,
        release,
        verbose,
        force,
        locked,
    };

    let result = konvoy_engine::build_tests(&root, &options)?;

    let profile = if release { "release" } else { "debug" };
    eprintln!(
        "    Finished `{profile}` test target in {:.2}s",
        result.compile_duration.as_secs_f64()
    );
    eprintln!("     Running `{}`", result.output_path.display());

    let mut cmd = std::process::Command::new(&result.output_path);
    if let Some(ref pattern) = filter {
        cmd.arg(format!("--ktest_filter={pattern}"));
    }

    let status = cmd
        .status()
        .map_err(|e| format!("cannot run {}: {e}", result.output_path.display()))?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        process::exit(code);
    }

    Ok(())
}

fn cmd_lint(verbose: bool, config: Option<PathBuf>, locked: bool) -> CliResult {
    let root = project_root()?;
    let options = konvoy_engine::LintOptions {
        verbose,
        config,
        locked,
    };

    let result = konvoy_engine::lint(&root, &options)?;

    if result.success {
        eprintln!("    No lint issues found");
        return Ok(());
    }

    if !verbose {
        for diag in &result.diagnostics {
            let location = match (&diag.file, diag.line) {
                (Some(f), Some(l)) => format!("{f}:{l}"),
                (Some(f), None) => f.clone(),
                _ => String::new(),
            };
            if location.is_empty() {
                eprintln!("  {}: {}", diag.rule, diag.message);
            } else {
                eprintln!("  {location}: {}: {}", diag.rule, diag.message);
            }
        }
    }
    eprintln!();
    Err(format!("lint found {} issue(s)", result.finding_count).into())
}

fn cmd_clean() -> CliResult {
    let root = project_root()?;
    let konvoy_dir = root.join(".konvoy");

    konvoy_util::fs::remove_dir_all_if_exists(&konvoy_dir)?;

    eprintln!("    Cleaned build artifacts");
    Ok(())
}

fn cmd_doctor() -> CliResult {
    eprintln!("Checking environment...");
    eprintln!();

    let mut issues = 0u32;

    // Check host target.
    match konvoy_targets::host_target() {
        Ok(target) => eprintln!("  [ok] Host target: {target}"),
        Err(e) => {
            eprintln!("  [!!] Host target: {e}");
            issues = issues.saturating_add(1);
        }
    }

    // Check for konvoy.toml in current directory and report managed toolchain status.
    let cwd = std::env::current_dir()?;
    if cwd.join("konvoy.toml").exists() {
        match konvoy_config::Manifest::from_path(&cwd.join("konvoy.toml")) {
            Ok(manifest) => {
                eprintln!("  [ok] Project: {}", manifest.package.name);
                let version = &manifest.toolchain.kotlin;
                match konvoy_konanc::toolchain::is_installed(version) {
                    Ok(true) => {
                        match konvoy_konanc::toolchain::managed_konanc_path(version) {
                            Ok(path) => eprintln!("  [ok] konanc: {version} ({})", path.display()),
                            Err(e) => {
                                eprintln!("  [!!] konanc: {e}");
                                issues = issues.saturating_add(1);
                            }
                        }
                        match konvoy_konanc::toolchain::jre_home_path(version) {
                            Ok(path) => eprintln!("  [ok] JRE: {}", path.display()),
                            Err(e) => {
                                eprintln!("  [!!] JRE: {e}");
                                issues = issues.saturating_add(1);
                            }
                        }
                    }
                    Ok(false) => {
                        eprintln!("  [!!] konanc: Kotlin/Native {version} not installed — run `konvoy toolchain install` or `konvoy build`");
                        issues = issues.saturating_add(1);
                    }
                    Err(e) => {
                        eprintln!("  [!!] konanc: {e}");
                        issues = issues.saturating_add(1);
                    }
                }

                // Check detekt availability if detekt is configured in [toolchain].
                if let Some(ref detekt_version) = manifest.toolchain.detekt {
                    match konvoy_engine::detekt::is_installed(detekt_version) {
                        Ok(true) => match konvoy_engine::detekt::detekt_jar_path(detekt_version) {
                            Ok(path) => {
                                eprintln!("  [ok] detekt: {detekt_version} ({})", path.display());
                            }
                            Err(e) => {
                                eprintln!("  [!!] detekt: {e}");
                                issues = issues.saturating_add(1);
                            }
                        },
                        Ok(false) => {
                            eprintln!("  [--] detekt: {detekt_version} not downloaded — will download on first `konvoy lint`");
                        }
                        Err(e) => {
                            eprintln!("  [!!] detekt: {e}");
                            issues = issues.saturating_add(1);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("  [!!] konvoy.toml: {e}");
                issues = issues.saturating_add(1);
            }
        }
    } else {
        eprintln!("  [--] No konvoy.toml in current directory");
        // Check if any managed toolchains are installed.
        match konvoy_konanc::toolchain::list_installed() {
            Ok(versions) if versions.is_empty() => {
                eprintln!("  [--] No managed toolchains installed");
            }
            Ok(versions) => {
                eprintln!("  [ok] Managed toolchains: {}", versions.join(", "));
            }
            Err(e) => {
                eprintln!("  [!!] Managed toolchains: {e}");
                issues = issues.saturating_add(1);
            }
        }
    }

    eprintln!();
    if issues > 0 {
        eprintln!("{issues} issue(s) found — fix them before building");
        Err(format!("{issues} issue(s) found").into())
    } else {
        eprintln!("All checks passed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use clap::CommandFactory;
    use clap::Parser;

    // ── Subcommand parsing ─────────────────────────────────────────

    #[test]
    fn parse_init_defaults() {
        let cli = Cli::try_parse_from(["konvoy", "init"]).unwrap();
        match cli.command {
            Command::Init { name, lib } => {
                assert!(name.is_none());
                assert!(!lib);
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_init_with_name() {
        let cli = Cli::try_parse_from(["konvoy", "init", "--name", "my-app"]).unwrap();
        match cli.command {
            Command::Init { name, lib } => {
                assert_eq!(name.as_deref(), Some("my-app"));
                assert!(!lib);
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_init_lib() {
        let cli = Cli::try_parse_from(["konvoy", "init", "--lib"]).unwrap();
        match cli.command {
            Command::Init { lib, .. } => assert!(lib),
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_init_name_and_lib() {
        let args = ["konvoy", "init", "--name", "mylib", "--lib"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Init { name, lib } => {
                assert_eq!(name.as_deref(), Some("mylib"));
                assert!(lib);
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn parse_build_defaults() {
        let cli = Cli::try_parse_from(["konvoy", "build"]).unwrap();
        match cli.command {
            Command::Build {
                target,
                release,
                verbose,
                force,
                locked,
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
                assert!(!force);
                assert!(!locked);
            }
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn parse_build_release() {
        let cli = Cli::try_parse_from(["konvoy", "build", "--release"]).unwrap();
        match cli.command {
            Command::Build { release, .. } => assert!(release),
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn parse_build_verbose() {
        let cli = Cli::try_parse_from(["konvoy", "build", "--verbose"]).unwrap();
        match cli.command {
            Command::Build { verbose, .. } => assert!(verbose),
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn parse_build_target() {
        let cli = Cli::try_parse_from(["konvoy", "build", "--target", "macos_arm64"]).unwrap();
        match cli.command {
            Command::Build { target, .. } => {
                assert_eq!(target.as_deref(), Some("macos_arm64"));
            }
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn parse_build_all_flags() {
        let cli = Cli::try_parse_from([
            "konvoy",
            "build",
            "--target",
            "linux_x64",
            "--release",
            "--verbose",
            "--force",
            "--locked",
        ])
        .unwrap();
        match cli.command {
            Command::Build {
                target,
                release,
                verbose,
                force,
                locked,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
                assert!(force);
                assert!(locked);
            }
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_defaults() {
        let cli = Cli::try_parse_from(["konvoy", "run"]).unwrap();
        match cli.command {
            Command::Run {
                target,
                release,
                verbose,
                force,
                locked,
                args,
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
                assert!(!force);
                assert!(!locked);
                assert!(args.is_empty());
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_verbose() {
        let cli = Cli::try_parse_from(["konvoy", "run", "--verbose"]).unwrap();
        match cli.command {
            Command::Run { verbose, .. } => assert!(verbose),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_verbose_short() {
        let cli = Cli::try_parse_from(["konvoy", "run", "-v"]).unwrap();
        match cli.command {
            Command::Run { verbose, .. } => assert!(verbose),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_all_flags() {
        let cli = Cli::try_parse_from([
            "konvoy",
            "run",
            "--target",
            "linux_x64",
            "--release",
            "--verbose",
            "--force",
            "--locked",
            "--",
            "arg1",
        ])
        .unwrap();
        match cli.command {
            Command::Run {
                target,
                release,
                verbose,
                force,
                locked,
                args,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
                assert!(force);
                assert!(locked);
                assert_eq!(args, vec!["arg1"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_with_passthrough_args() {
        let args = ["konvoy", "run", "--", "arg1", "arg2", "--flag"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Run { args, force, .. } => {
                assert_eq!(args, vec!["arg1", "arg2", "--flag"]);
                assert!(!force);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_release_with_passthrough() {
        let args = ["konvoy", "run", "--release", "--", "hello"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Run { release, args, .. } => {
                assert!(release);
                assert_eq!(args, vec!["hello"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_target_and_release() {
        let args = ["konvoy", "run", "--target", "macos_x64", "--release"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Run {
                target, release, ..
            } => {
                assert_eq!(target.as_deref(), Some("macos_x64"));
                assert!(release);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_test_defaults() {
        let cli = Cli::try_parse_from(["konvoy", "test"]).unwrap();
        match cli.command {
            Command::Test {
                target,
                release,
                verbose,
                force,
                locked,
                filter,
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
                assert!(!force);
                assert!(!locked);
                assert!(filter.is_none());
            }
            other => panic!("expected Test, got {other:?}"),
        }
    }

    #[test]
    fn parse_test_all_flags() {
        let cli = Cli::try_parse_from([
            "konvoy",
            "test",
            "--release",
            "--verbose",
            "--target",
            "linux_x64",
            "--force",
            "--locked",
            "--filter",
            "MathTest.*",
        ])
        .unwrap();
        match cli.command {
            Command::Test {
                target,
                release,
                verbose,
                force,
                locked,
                filter,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
                assert!(force);
                assert!(locked);
                assert_eq!(filter.as_deref(), Some("MathTest.*"));
            }
            other => panic!("expected Test, got {other:?}"),
        }
    }

    #[test]
    fn parse_clean() {
        let cli = Cli::try_parse_from(["konvoy", "clean"]).unwrap();
        assert!(matches!(cli.command, Command::Clean));
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::try_parse_from(["konvoy", "doctor"]).unwrap();
        assert!(matches!(cli.command, Command::Doctor));
    }

    #[test]
    fn parse_toolchain_install_with_version() {
        let args = ["konvoy", "toolchain", "install", "2.1.0"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Toolchain {
                action: ToolchainAction::Install { version },
            } => {
                assert_eq!(version.as_deref(), Some("2.1.0"));
            }
            other => panic!("expected Toolchain Install, got {other:?}"),
        }
    }

    #[test]
    fn parse_toolchain_install_no_version() {
        let cli = Cli::try_parse_from(["konvoy", "toolchain", "install"]).unwrap();
        match cli.command {
            Command::Toolchain {
                action: ToolchainAction::Install { version },
            } => {
                assert!(version.is_none());
            }
            other => panic!("expected Toolchain Install, got {other:?}"),
        }
    }

    #[test]
    fn parse_toolchain_list() {
        let cli = Cli::try_parse_from(["konvoy", "toolchain", "list"]).unwrap();
        match cli.command {
            Command::Toolchain {
                action: ToolchainAction::List,
            } => {}
            other => panic!("expected Toolchain List, got {other:?}"),
        }
    }

    // ── Flag order independence ────────────────────────────────────

    #[test]
    fn build_flags_order_verbose_before_release() {
        let args = ["konvoy", "build", "--verbose", "--release"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Build {
                release, verbose, ..
            } => {
                assert!(release);
                assert!(verbose);
            }
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn build_flags_order_target_between() {
        let cli = Cli::try_parse_from([
            "konvoy",
            "build",
            "--release",
            "--target",
            "linux_x64",
            "--verbose",
        ])
        .unwrap();
        match cli.command {
            Command::Build {
                target,
                release,
                verbose,
                ..
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
            }
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn init_flags_order_lib_before_name() {
        let args = ["konvoy", "init", "--lib", "--name", "foo"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Init { name, lib } => {
                assert_eq!(name.as_deref(), Some("foo"));
                assert!(lib);
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    // ── Invalid arguments ──────────────────────────────────────────

    #[test]
    fn error_no_subcommand() {
        let err = Cli::try_parse_from(["konvoy"]).unwrap_err();
        let expected = ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand;
        assert_eq!(err.kind(), expected);
    }

    #[test]
    fn error_unknown_subcommand() {
        let err = Cli::try_parse_from(["konvoy", "deploy"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn error_unknown_flag_on_build() {
        let err = Cli::try_parse_from(["konvoy", "build", "--optimize"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
        let msg = err.to_string();
        assert!(msg.contains("--optimize"));
        assert!(msg.contains("Usage:"));
    }

    #[test]
    fn error_target_missing_value() {
        let err = Cli::try_parse_from(["konvoy", "build", "--target"]).unwrap_err();
        // clap reports this as either invalid or missing argument depending on version.
        assert!(
            err.kind() == ErrorKind::InvalidValue
                || err.kind() == ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn error_unknown_flag_on_init() {
        let err = Cli::try_parse_from(["konvoy", "init", "--force"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn error_unknown_toolchain_action() {
        let args = ["konvoy", "toolchain", "remove"];
        let err = Cli::try_parse_from(args).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
        let msg = err.to_string();
        assert!(msg.contains("remove"));
        assert!(msg.contains("Usage:"));
    }

    #[test]
    fn error_clean_takes_no_args() {
        let err = Cli::try_parse_from(["konvoy", "clean", "--all"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn error_doctor_takes_no_args() {
        let err = Cli::try_parse_from(["konvoy", "doctor", "--fix"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    // ── Help and version output ────────────────────────────────────

    #[test]
    fn help_flag_on_root() {
        let err = Cli::try_parse_from(["konvoy", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        let output = err.to_string();
        assert!(output.contains("A native-first Kotlin build tool"));
        assert!(output.contains("Commands:"));
        assert!(output.contains("build"));
        assert!(output.contains("toolchain"));
    }

    #[test]
    fn help_flag_on_build() {
        let err = Cli::try_parse_from(["konvoy", "build", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_init() {
        let err = Cli::try_parse_from(["konvoy", "init", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_run() {
        let err = Cli::try_parse_from(["konvoy", "run", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_test() {
        let err = Cli::try_parse_from(["konvoy", "test", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_clean() {
        let err = Cli::try_parse_from(["konvoy", "clean", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_doctor() {
        let err = Cli::try_parse_from(["konvoy", "doctor", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_toolchain() {
        let args = ["konvoy", "toolchain", "--help"];
        let err = Cli::try_parse_from(args).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_toolchain_install() {
        let args = ["konvoy", "toolchain", "install", "--help"];
        let err = Cli::try_parse_from(args).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_toolchain_list() {
        let args = ["konvoy", "toolchain", "list", "--help"];
        let err = Cli::try_parse_from(args).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        let output = err.to_string();
        assert!(output.contains("List installed Kotlin/Native versions"));
    }

    #[test]
    fn version_flag() {
        let err = Cli::try_parse_from(["konvoy", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn root_help_render_includes_all_subcommands() {
        let mut cmd = Cli::command();
        let help = cmd.render_help().to_string();
        for subcommand in [
            "init",
            "build",
            "run",
            "test",
            "lint",
            "clean",
            "doctor",
            "toolchain",
        ] {
            assert!(help.contains(subcommand));
        }
    }

    // ── Passthrough edge cases ─────────────────────────────────────

    #[test]
    fn run_empty_passthrough() {
        let cli = Cli::try_parse_from(["konvoy", "run", "--"]).unwrap();
        match cli.command {
            Command::Run { args, .. } => assert!(args.is_empty()),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn run_passthrough_with_dashes() {
        let args = ["konvoy", "run", "--", "--verbose", "--release"];
        let cli = Cli::try_parse_from(args).unwrap();
        match cli.command {
            Command::Run { args, .. } => {
                assert_eq!(args, vec!["--verbose", "--release"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    // ── Lint parsing ─────────────────────────────────────────────────

    #[test]
    fn parse_lint_defaults() {
        let cli = Cli::try_parse_from(["konvoy", "lint"]).unwrap();
        match cli.command {
            Command::Lint {
                verbose,
                config,
                locked,
            } => {
                assert!(!verbose);
                assert!(config.is_none());
                assert!(!locked);
            }
            other => panic!("expected Lint, got {other:?}"),
        }
    }

    #[test]
    fn parse_lint_verbose() {
        let cli = Cli::try_parse_from(["konvoy", "lint", "--verbose"]).unwrap();
        match cli.command {
            Command::Lint { verbose, .. } => assert!(verbose),
            other => panic!("expected Lint, got {other:?}"),
        }
    }

    #[test]
    fn parse_lint_with_config() {
        let cli = Cli::try_parse_from(["konvoy", "lint", "--config", "my-detekt.yml"]).unwrap();
        match cli.command {
            Command::Lint { config, .. } => {
                assert_eq!(config, Some(PathBuf::from("my-detekt.yml")));
            }
            other => panic!("expected Lint, got {other:?}"),
        }
    }

    #[test]
    fn parse_lint_all_flags() {
        let cli = Cli::try_parse_from([
            "konvoy",
            "lint",
            "--verbose",
            "--config",
            "custom.yml",
            "--locked",
        ])
        .unwrap();
        match cli.command {
            Command::Lint {
                verbose,
                config,
                locked,
            } => {
                assert!(verbose);
                assert_eq!(config, Some(PathBuf::from("custom.yml")));
                assert!(locked);
            }
            other => panic!("expected Lint, got {other:?}"),
        }
    }

    #[test]
    fn help_flag_on_lint() {
        let err = Cli::try_parse_from(["konvoy", "lint", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }
}

fn cmd_toolchain(action: ToolchainAction) -> CliResult {
    match action {
        ToolchainAction::Install { version } => {
            let version = if let Some(v) = version {
                v
            } else {
                // Read version from konvoy.toml in current directory.
                let cwd = std::env::current_dir()?;
                let manifest_path = cwd.join("konvoy.toml");
                let manifest = konvoy_config::Manifest::from_path(&manifest_path)?;
                manifest.toolchain.kotlin
            };

            match konvoy_konanc::toolchain::is_installed(&version) {
                Ok(true) => {
                    eprintln!("    Kotlin/Native {version} is already installed");
                    return Ok(());
                }
                Ok(false) => {}
                Err(e) => return Err(e.into()),
            }

            eprintln!("    Installing Kotlin/Native {version}...");
            let result = konvoy_konanc::toolchain::install(&version)?;
            eprintln!(
                "    Installed Kotlin/Native {version} at {}",
                result.konanc_path.display()
            );
            Ok(())
        }
        ToolchainAction::List => {
            let versions = konvoy_konanc::toolchain::list_installed()?;
            if versions.is_empty() {
                eprintln!("No toolchains installed");
                eprintln!();
                eprintln!("  Install one with: konvoy toolchain install <version>");
            } else {
                eprintln!("Installed toolchains:");
                for v in &versions {
                    eprintln!("  {v}");
                }
            }
            Ok(())
        }
    }
}
