use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "konvoy", about = "A native-first Kotlin build tool")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
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

#[derive(Subcommand)]
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
