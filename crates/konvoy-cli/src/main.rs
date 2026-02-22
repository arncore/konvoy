#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

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
    },
    /// Build and run the project
    Run {
        /// Target triple (defaults to host)
        #[arg(long)]
        target: Option<String>,
        /// Run in release mode
        #[arg(long)]
        release: bool,
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
        } => cmd_build(target, release, verbose),
        Command::Run {
            target,
            release,
            args,
        } => cmd_run(target, release, &args),
        Command::Test {
            target,
            release,
            verbose,
        } => cmd_test(target, release, verbose),
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
fn project_root() -> Result<PathBuf, String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine working directory: {e}"))?;
    let manifest = cwd.join("konvoy.toml");
    if !manifest.exists() {
        return Err(
            "no konvoy.toml found in current directory — run `konvoy init` to create a project"
                .to_owned(),
        );
    }
    Ok(cwd)
}

fn cmd_init(name: Option<String>, lib: bool) -> Result<(), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine working directory: {e}"))?;

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

    konvoy_engine::init_project_with_kind(&project_name, &project_dir, kind)
        .map_err(|e| e.to_string())?;

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

fn cmd_build(target: Option<String>, release: bool, verbose: bool) -> Result<(), String> {
    let root = project_root()?;
    let options = konvoy_engine::BuildOptions {
        target,
        release,
        verbose,
    };

    let result = konvoy_engine::build(&root, &options).map_err(|e| e.to_string())?;

    let profile = if release { "release" } else { "debug" };
    match result.outcome {
        konvoy_engine::BuildOutcome::Cached => {
            eprintln!(
                "    Finished `{profile}` target in {:.2}s",
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

fn cmd_run(target: Option<String>, release: bool, args: &[String]) -> Result<(), String> {
    let root = project_root()?;

    // Cannot run a library project.
    let manifest =
        konvoy_config::Manifest::from_path(&root.join("konvoy.toml")).map_err(|e| e.to_string())?;
    if manifest.package.kind == konvoy_config::manifest::PackageKind::Lib {
        return Err(
            "cannot run a library project — only binary projects (kind = \"bin\") can be run"
                .to_owned(),
        );
    }

    let options = konvoy_engine::BuildOptions {
        target,
        release,
        verbose: false,
    };

    let result = konvoy_engine::build(&root, &options).map_err(|e| e.to_string())?;

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

fn cmd_test(target: Option<String>, release: bool, verbose: bool) -> Result<(), String> {
    // For MVP, konvoy test is essentially build + run with a test convention.
    // Kotlin/Native doesn't have a built-in test framework like cargo test,
    // so for now we compile and run the project and report the result.
    eprintln!("    Note: Kotlin/Native test support is minimal — running the project as a test");

    let root = project_root()?;
    let options = konvoy_engine::BuildOptions {
        target,
        release,
        verbose,
    };

    let result = konvoy_engine::build(&root, &options).map_err(|e| e.to_string())?;

    let profile = if release { "release" } else { "debug" };
    eprintln!(
        "    Finished `{profile}` target in {:.2}s",
        result.duration.as_secs_f64()
    );
    eprintln!("     Running `{}`", result.output_path.display());

    let status = std::process::Command::new(&result.output_path)
        .status()
        .map_err(|e| format!("cannot run {}: {e}", result.output_path.display()))?;

    if status.success() {
        eprintln!("    Test passed");
    } else {
        eprintln!("    Test failed");
        let code = status.code().unwrap_or(1);
        process::exit(code);
    }

    Ok(())
}

fn cmd_clean() -> Result<(), String> {
    let root = project_root()?;
    let konvoy_dir = root.join(".konvoy");

    konvoy_util::fs::remove_dir_all_if_exists(&konvoy_dir).map_err(|e| e.to_string())?;

    eprintln!("    Cleaned build artifacts");
    Ok(())
}

fn cmd_doctor() -> Result<(), String> {
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
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine working directory: {e}"))?;
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
        Err(format!("{issues} issue(s) found"))
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
        let cli =
            Cli::try_parse_from(["konvoy", "init", "--name", "mylib", "--lib"]).unwrap();
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
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
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
        ])
        .unwrap();
        match cli.command {
            Command::Build {
                target,
                release,
                verbose,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
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
                args,
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(args.is_empty());
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_with_passthrough_args() {
        let cli =
            Cli::try_parse_from(["konvoy", "run", "--", "arg1", "arg2", "--flag"]).unwrap();
        match cli.command {
            Command::Run { args, .. } => {
                assert_eq!(args, vec!["arg1", "arg2", "--flag"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_release_with_passthrough() {
        let cli = Cli::try_parse_from([
            "konvoy", "run", "--release", "--", "hello",
        ])
        .unwrap();
        match cli.command {
            Command::Run {
                release, args, ..
            } => {
                assert!(release);
                assert_eq!(args, vec!["hello"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parse_run_target_and_release() {
        let cli = Cli::try_parse_from([
            "konvoy",
            "run",
            "--target",
            "macos_x64",
            "--release",
        ])
        .unwrap();
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
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
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
        ])
        .unwrap();
        match cli.command {
            Command::Test {
                target,
                release,
                verbose,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
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
        let cli =
            Cli::try_parse_from(["konvoy", "toolchain", "install", "2.1.0"]).unwrap();
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
        let cli =
            Cli::try_parse_from(["konvoy", "build", "--verbose", "--release"]).unwrap();
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
        let cli =
            Cli::try_parse_from(["konvoy", "init", "--lib", "--name", "foo"]).unwrap();
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
        assert_eq!(err.kind(), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
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
        let err =
            Cli::try_parse_from(["konvoy", "toolchain", "remove"]).unwrap_err();
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
        let err =
            Cli::try_parse_from(["konvoy", "toolchain", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_toolchain_install() {
        let err =
            Cli::try_parse_from(["konvoy", "toolchain", "install", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_toolchain_list() {
        let err =
            Cli::try_parse_from(["konvoy", "toolchain", "list", "--help"]).unwrap_err();
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
        let cli = Cli::try_parse_from([
            "konvoy", "run", "--", "--verbose", "--release",
        ])
        .unwrap();
        match cli.command {
            Command::Run { args, .. } => {
                assert_eq!(args, vec!["--verbose", "--release"]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }
}

fn cmd_toolchain(action: ToolchainAction) -> Result<(), String> {
    match action {
        ToolchainAction::Install { version } => {
            let version = if let Some(v) = version {
                v
            } else {
                // Read version from konvoy.toml in current directory.
                let cwd = std::env::current_dir()
                    .map_err(|e| format!("cannot determine working directory: {e}"))?;
                let manifest_path = cwd.join("konvoy.toml");
                let manifest = konvoy_config::Manifest::from_path(&manifest_path)
                    .map_err(|e| e.to_string())?;
                manifest.toolchain.kotlin
            };

            match konvoy_konanc::toolchain::is_installed(&version) {
                Ok(true) => {
                    eprintln!("    Kotlin/Native {version} is already installed");
                    return Ok(());
                }
                Ok(false) => {}
                Err(e) => return Err(e.to_string()),
            }

            eprintln!("    Installing Kotlin/Native {version}...");
            let result = konvoy_konanc::toolchain::install(&version).map_err(|e| e.to_string())?;
            eprintln!(
                "    Installed Kotlin/Native {version} at {}",
                result.konanc_path.display()
            );
            Ok(())
        }
        ToolchainAction::List => {
            let versions = konvoy_konanc::toolchain::list_installed().map_err(|e| e.to_string())?;
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
