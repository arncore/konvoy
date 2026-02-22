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
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Init { name } => cmd_init(name),
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

fn cmd_init(name: Option<String>) -> Result<(), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine working directory: {e}"))?;

    let project_name = name.unwrap_or_else(|| {
        cwd.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("my-project")
            .to_owned()
    });

    let project_dir = cwd.join(&project_name);

    konvoy_engine::init_project(&project_name, &project_dir).map_err(|e| e.to_string())?;

    eprintln!(
        "    Created project `{project_name}` at {}",
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

    // Check konanc.
    match konvoy_konanc::detect_konanc() {
        Ok(info) => {
            eprintln!("  [ok] konanc: {} ({})", info.version, info.path.display());
        }
        Err(e) => {
            eprintln!("  [!!] konanc: {e}");
            issues = issues.saturating_add(1);
        }
    }

    // Check for konvoy.toml in current directory.
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine working directory: {e}"))?;
    if cwd.join("konvoy.toml").exists() {
        match konvoy_config::Manifest::from_path(&cwd.join("konvoy.toml")) {
            Ok(manifest) => eprintln!("  [ok] Project: {}", manifest.package.name),
            Err(e) => {
                eprintln!("  [!!] konvoy.toml: {e}");
                issues = issues.saturating_add(1);
            }
        }
    } else {
        eprintln!("  [--] No konvoy.toml in current directory");
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
