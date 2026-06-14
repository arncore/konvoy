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
        #[arg(long, short = 'v')]
        verbose: bool,
        /// Force a rebuild, bypassing the cache
        #[arg(long)]
        force: bool,
        /// Assert that konvoy.lock is up to date and never modify it (pinned
        /// artifacts may still be downloaded; only lockfile drift is an error)
        #[arg(long)]
        locked: bool,
        /// Run without network access: every managed artifact must already be
        /// present locally, or the build fails
        #[arg(long)]
        offline: bool,
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
        /// Assert that konvoy.lock is up to date and never modify it (pinned
        /// artifacts may still be downloaded; only lockfile drift is an error)
        #[arg(long)]
        locked: bool,
        /// Run without network access: every managed artifact must already be
        /// present locally, or the build fails
        #[arg(long)]
        offline: bool,
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
        #[arg(long, short = 'v')]
        verbose: bool,
        /// Force a rebuild, bypassing the cache
        #[arg(long)]
        force: bool,
        /// Assert that konvoy.lock is up to date and never modify it (pinned
        /// artifacts may still be downloaded; only lockfile drift is an error)
        #[arg(long)]
        locked: bool,
        /// Run without network access: every managed artifact must already be
        /// present locally, or the build fails
        #[arg(long)]
        offline: bool,
        /// Only run tests matching this pattern (forwarded to --ktest_filter)
        #[arg(long)]
        filter: Option<String>,
    },
    /// Run detekt linter on Kotlin source files
    Lint {
        /// Show raw detekt output
        #[arg(long, short = 'v')]
        verbose: bool,
        /// Path to a custom detekt configuration file
        #[arg(long)]
        config: Option<PathBuf>,
        /// Assert that konvoy.lock is up to date and never modify it (the pinned
        /// detekt JAR may still be downloaded; only lockfile drift is an error)
        #[arg(long)]
        locked: bool,
        /// Run without network access: detekt and its JRE must already be
        /// present locally, or the lint fails
        #[arg(long)]
        offline: bool,
    },
    /// Run code generators (e.g. OpenAPI/Fabrikt) without compiling
    Generate {
        /// Show raw generator output
        #[arg(long, short = 'v')]
        verbose: bool,
        /// Assert that konvoy.lock is up to date and never modify it (pinned
        /// codegen tools may still be downloaded; only lockfile drift is an error)
        #[arg(long)]
        locked: bool,
        /// Run without network access: every codegen tool and the JRE must
        /// already be present locally, or generation fails
        #[arg(long)]
        offline: bool,
    },
    /// Resolve Maven dependencies and update konvoy.lock
    Update,
    /// Remove build artifacts
    Clean {
        /// Remove the entire .konvoy/ directory, not just build artifacts
        #[arg(long)]
        all: bool,
    },
    /// Check environment and toolchain setup
    Doctor,
    /// Validate konvoy.toml and report configuration issues
    Check {
        /// Output format: human-readable text, or JSON (for editors/tools)
        #[arg(long, value_enum, default_value_t = CheckFormat::Human)]
        format: CheckFormat,
    },
    /// Manage Kotlin/Native toolchains
    Toolchain {
        #[command(subcommand)]
        action: ToolchainAction,
    },
}

/// Output format for `konvoy check`.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum CheckFormat {
    /// Human-readable lines on stderr (non-zero exit if issues are found).
    Human,
    /// A JSON array of diagnostics on stdout (always exits 0 — the issues are the
    /// data). Stable contract for editor integrations.
    Json,
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

/// Build a command-scoped `ArtifactResolver` from the `--offline`/`--locked`
/// flags and hand it to `f`.
///
/// The resolver borrows a `NetworkClient` that lives only for this call, so it is
/// threaded through a closure rather than returned. Every fetching command builds
/// its resolver this way, keeping the net/lockfile wiring in one place.
fn with_resolver<T>(
    offline: bool,
    locked: bool,
    f: impl FnOnce(konvoy_engine::ArtifactResolver<'_>) -> T,
) -> T {
    let net = konvoy_util::net::NetworkClient::new(offline);
    let resolver =
        konvoy_engine::ArtifactResolver::new(&net, konvoy_engine::LockfileManager::new(locked));
    f(resolver)
}

fn main() {
    let cli = Cli::parse();

    // The single outbound-HTTP funnel for the whole process: one client per
    // invocation, built here at the program entry from the command's --offline
    // flag (or always-online for inherently-online commands) and threaded into
    // everything that may fetch. `--offline` lives in the client, not in the
    // build/lint options — network access is the client's concern.
    let result = match cli.command {
        Command::Init { name, lib } => cmd_init(name, lib),
        Command::Build {
            target,
            release,
            verbose,
            force,
            locked,
            offline,
        } => with_resolver(offline, locked, |resolver| {
            cmd_build(target, profile_from_flag(release), verbose, force, resolver)
        }),
        Command::Run {
            target,
            release,
            verbose,
            force,
            locked,
            offline,
            args,
        } => with_resolver(offline, locked, |resolver| {
            cmd_run(
                target,
                profile_from_flag(release),
                verbose,
                force,
                &args,
                resolver,
            )
        }),
        Command::Test {
            target,
            release,
            verbose,
            force,
            locked,
            offline,
            filter,
        } => with_resolver(offline, locked, |resolver| {
            cmd_test(
                target,
                profile_from_flag(release),
                verbose,
                force,
                &filter,
                resolver,
            )
        }),
        Command::Lint {
            verbose,
            config,
            locked,
            offline,
        } => with_resolver(offline, locked, |resolver| {
            cmd_lint(verbose, config, resolver)
        }),
        Command::Generate {
            verbose,
            locked,
            offline,
        } => with_resolver(offline, locked, |resolver| cmd_generate(verbose, resolver)),
        // `konvoy update` is inherently online and never locked: it exists to
        // (re)resolve dependencies and rewrite konvoy.lock.
        Command::Update => with_resolver(false, false, cmd_update),
        Command::Clean { all } => cmd_clean(all),
        Command::Doctor => cmd_doctor(),
        Command::Check { format } => cmd_check(format),
        Command::Toolchain { action } => {
            cmd_toolchain(action, &konvoy_util::net::NetworkClient::new(false))
        }
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

    let kind = if lib {
        konvoy_config::manifest::PackageKind::Lib
    } else {
        konvoy_config::manifest::PackageKind::Bin
    };

    let kind_label = if lib { "library" } else { "project" };

    if let Some(project_name) = name {
        // `konvoy init --name <name>`: create a subdirectory.
        let project_dir = cwd.join(&project_name);
        konvoy_engine::init_project_with_kind(&project_name, &project_dir, kind)?;

        eprintln!(
            "    Created {kind_label} `{project_name}` at {}",
            project_dir.display()
        );
        eprintln!();
        eprintln!("  To get started:");
        eprintln!("    cd {project_name}");
        eprintln!("    konvoy build");
    } else {
        // `konvoy init` (no --name): initialize in the current directory.
        let project_name = konvoy_engine::init_project_in_place(&cwd, kind)?;

        eprintln!(
            "    Created {kind_label} `{project_name}` at {}",
            cwd.display()
        );
        eprintln!();
        eprintln!("  To get started:");
        eprintln!("    konvoy build");
    }

    Ok(())
}

/// Map the `--release` CLI flag to a `Profile` at the boundary.
fn profile_from_flag(release: bool) -> konvoy_config::Profile {
    if release {
        konvoy_config::Profile::Release
    } else {
        konvoy_config::Profile::Debug
    }
}

fn build_options(
    target: Option<String>,
    profile: konvoy_config::Profile,
    verbose: bool,
    force: bool,
) -> konvoy_engine::BuildOptions {
    konvoy_engine::BuildOptions {
        target,
        profile,
        verbose,
        force,
    }
}

fn cmd_build(
    target: Option<String>,
    profile: konvoy_config::Profile,
    verbose: bool,
    force: bool,
    resolver: konvoy_engine::ArtifactResolver<'_>,
) -> CliResult {
    let root = project_root()?;
    let options = build_options(target, profile, verbose, force);

    let result = konvoy_engine::build(&root, &options, resolver)?;

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
    profile: konvoy_config::Profile,
    verbose: bool,
    force: bool,
    args: &[String],
    resolver: konvoy_engine::ArtifactResolver<'_>,
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

    let options = build_options(target, profile, verbose, force);

    let result = konvoy_engine::build(&root, &options, resolver)?;

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
    profile: konvoy_config::Profile,
    verbose: bool,
    force: bool,
    filter: &Option<String>,
    resolver: konvoy_engine::ArtifactResolver<'_>,
) -> CliResult {
    let root = project_root()?;
    let options = build_options(target, profile, verbose, force);

    let result = konvoy_engine::build_tests(&root, &options, resolver)?;

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

fn cmd_lint(
    verbose: bool,
    config: Option<PathBuf>,
    resolver: konvoy_engine::ArtifactResolver<'_>,
) -> CliResult {
    let root = project_root()?;
    let options = konvoy_engine::LintOptions { verbose, config };

    let result = konvoy_engine::lint(&root, &options, resolver)?;

    if result.success {
        eprintln!("    No lint issues found");
        return Ok(());
    }

    if !verbose {
        for diag in &result.diagnostics {
            match (&diag.file, diag.line) {
                (Some(f), Some(l)) => eprintln!("  {f}:{l}: {}: {}", diag.rule, diag.message),
                (Some(f), None) => eprintln!("  {f}: {}: {}", diag.rule, diag.message),
                _ => eprintln!("  {}: {}", diag.rule, diag.message),
            }
        }
    }
    eprintln!();
    Err(format!("lint found {} issue(s)", result.finding_count).into())
}

fn cmd_generate(verbose: bool, resolver: konvoy_engine::ArtifactResolver<'_>) -> CliResult {
    let root = project_root()?;

    let result = konvoy_engine::generate(&root, verbose, resolver)?;

    for output in &result.outputs {
        eprintln!(
            "    Generated {} file(s) for {} \u{2192} {}",
            output.file_count,
            output.display_name,
            output.output_dir.display()
        );
    }
    Ok(())
}

fn cmd_update(resolver: konvoy_engine::ArtifactResolver<'_>) -> CliResult {
    let root = project_root()?;
    // `konvoy update` is inherently online — resolving fetches POMs/klibs.
    let result = konvoy_engine::update(&root, resolver)?;
    eprintln!(
        "  Updated {} dependencies in konvoy.lock",
        result.updated_count
    );
    Ok(())
}

fn cmd_clean(all: bool) -> CliResult {
    let root = project_root()?;
    clean_project(&root, all)
}

fn clean_project(root: &std::path::Path, all: bool) -> CliResult {
    let konvoy_dir = root.join(".konvoy");

    if all {
        konvoy_util::fs::remove_dir_all_if_exists(&konvoy_dir)?;
        eprintln!("    Removed .konvoy/");
    } else {
        let build_dir = konvoy_dir.join("build");
        konvoy_util::fs::remove_dir_all_if_exists(&build_dir)?;
        eprintln!("    Removed build artifacts");
    }

    Ok(())
}

fn check_host_target() -> u32 {
    match konvoy_targets::host_target() {
        Ok(target) => {
            eprintln!("  [ok] Host target: {target}");
            0
        }
        Err(e) => {
            eprintln!("  [!!] Host target: {e}");
            1
        }
    }
}

fn check_toolchain(manifest: &konvoy_config::Manifest) -> u32 {
    let mut issues = 0u32;
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
    issues
}

fn check_detekt(manifest: &konvoy_config::Manifest) -> u32 {
    let Some(ref detekt_version) = manifest.toolchain.detekt else {
        return 0;
    };
    match konvoy_engine::detekt::is_installed(detekt_version) {
        Ok(true) => match konvoy_engine::detekt::detekt_jar_path(detekt_version) {
            Ok(path) => {
                eprintln!("  [ok] detekt: {detekt_version} ({})", path.display());
                0
            }
            Err(e) => {
                eprintln!("  [!!] detekt: {e}");
                1
            }
        },
        Ok(false) => {
            eprintln!("  [--] detekt: {detekt_version} not downloaded — will download on first `konvoy lint`");
            0
        }
        Err(e) => {
            eprintln!("  [!!] detekt: {e}");
            1
        }
    }
}

/// Report the install status of each configured codegen tool. A not-yet-downloaded
/// tool is informational (`[--]`, 0 issues) — it downloads on first use, like
/// detekt; only a failure to inspect it counts as an issue.
fn check_codegen(manifest: &konvoy_config::Manifest) -> u32 {
    let mut issues = 0u32;
    for generator in konvoy_engine::codegen::active_generators(&manifest.codegen) {
        let tool = generator.managed_tool();
        let label = generator.display_name();
        // `artifact_path()` is the same path `is_installed()` checks; compute it
        // once and `.exists()` it, rather than resolving the path twice.
        match tool.artifact_path() {
            Ok(path) if path.exists() => eprintln!(
                "  [ok] {label} ({}): {} ({})",
                tool.id(),
                tool.version(),
                path.display()
            ),
            Ok(_) => eprintln!(
                "  [--] {label} ({}): {} not downloaded — will download on first `konvoy generate` or `konvoy build`",
                tool.id(),
                tool.version()
            ),
            Err(e) => {
                eprintln!("  [!!] {label} ({}): {e}", tool.id());
                issues = issues.saturating_add(1);
            }
        }
    }
    issues
}

fn check_maven_deps(manifest: &konvoy_config::Manifest, cwd: &std::path::Path) -> u32 {
    let maven_deps: Vec<_> = manifest
        .dependencies
        .iter()
        .filter(|(_, spec)| spec.is_maven())
        .collect();

    if maven_deps.is_empty() {
        return 0;
    }

    let mut issues = 0u32;

    for (dep_name, dep_spec) in &maven_deps {
        if let (Some(ref maven), Some(ref dep_version)) = (&dep_spec.maven, &dep_spec.version) {
            eprintln!("  [ok] Maven dep: {} {} ({})", dep_name, dep_version, maven);
        }
    }

    let lockfile_path = cwd.join("konvoy.lock");
    if !lockfile_path.exists() {
        eprintln!(
            "  [!!] No konvoy.lock found — run 'konvoy update' to resolve Maven dependencies"
        );
        return 1;
    }

    match konvoy_config::lockfile::Lockfile::from_path(&lockfile_path) {
        Ok(lockfile) => {
            for (dep_name, _) in &maven_deps {
                if lockfile.has_maven_entry(dep_name) {
                    eprintln!("  [ok] Lockfile entry: {}", dep_name);
                } else {
                    eprintln!(
                        "  [!!] Lockfile entry: '{}' not found — run 'konvoy update'",
                        dep_name
                    );
                    issues = issues.saturating_add(1);
                }
            }
        }
        Err(e) => {
            eprintln!("  [!!] Lockfile: {}", e);
            issues = issues.saturating_add(1);
        }
    }

    issues
}

fn check_standalone_toolchains() -> u32 {
    match konvoy_konanc::toolchain::list_installed() {
        Ok(versions) if versions.is_empty() => {
            eprintln!("  [--] No managed toolchains installed");
            0
        }
        Ok(versions) => {
            eprintln!("  [ok] Managed toolchains: {}", versions.join(", "));
            0
        }
        Err(e) => {
            eprintln!("  [!!] Managed toolchains: {e}");
            1
        }
    }
}

fn cmd_doctor() -> CliResult {
    eprintln!("Checking environment...");
    eprintln!();

    let mut issues = check_host_target();

    let cwd = std::env::current_dir()?;
    if cwd.join("konvoy.toml").exists() {
        match konvoy_config::Manifest::from_path(&cwd.join("konvoy.toml")) {
            Ok(manifest) => {
                eprintln!("  [ok] Project: {}", manifest.package.name);
                issues = issues.saturating_add(check_toolchain(&manifest));
                issues = issues.saturating_add(check_detekt(&manifest));
                issues = issues.saturating_add(check_codegen(&manifest));
                issues = issues.saturating_add(check_maven_deps(&manifest, &cwd));
            }
            Err(e) => {
                eprintln!("  [!!] konvoy.toml: {e}");
                issues = issues.saturating_add(1);
            }
        }
    } else {
        eprintln!("  [--] No konvoy.toml in current directory");
        issues = issues.saturating_add(check_standalone_toolchains());
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

fn cmd_check(format: CheckFormat) -> CliResult {
    let root = project_root()?;
    let path = root.join("konvoy.toml");
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let diagnostics = konvoy_config::Manifest::check_str(&content, &path.display().to_string());

    match format {
        // Machine contract for editors: a JSON array on stdout, always exit 0 — the
        // diagnostics are the payload, not a process failure.
        CheckFormat::Json => {
            let json = serde_json::to_string(&diagnostics)
                .map_err(|e| format!("cannot serialize diagnostics: {e}"))?;
            println!("{json}");
            Ok(())
        }
        CheckFormat::Human => {
            if diagnostics.is_empty() {
                eprintln!("    No issues found in konvoy.toml");
                return Ok(());
            }
            for d in &diagnostics {
                let loc = match (d.line, d.column) {
                    (Some(line), Some(col)) => format!("konvoy.toml:{line}:{col}: "),
                    _ => String::new(),
                };
                eprintln!("  {loc}{}", d.message);
            }
            Err(format!("{} issue(s) found in konvoy.toml", diagnostics.len()).into())
        }
    }
}

fn cmd_toolchain(action: ToolchainAction, net: &konvoy_util::net::NetworkClient) -> CliResult {
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
            let result = konvoy_konanc::toolchain::install(&version, net)?;
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
                offline,
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
                assert!(!force);
                assert!(!locked);
                assert!(!offline);
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
    fn parse_build_verbose_short() {
        let cli = Cli::try_parse_from(["konvoy", "build", "-v"]).unwrap();
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
            "--offline",
        ])
        .unwrap();
        match cli.command {
            Command::Build {
                target,
                release,
                verbose,
                force,
                locked,
                offline,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
                assert!(force);
                assert!(locked);
                assert!(offline);
            }
            other => panic!("expected Build, got {other:?}"),
        }
    }

    #[test]
    fn parse_build_offline() {
        let cli = Cli::try_parse_from(["konvoy", "build", "--offline"]).unwrap();
        match cli.command {
            Command::Build {
                locked, offline, ..
            } => {
                assert!(offline);
                assert!(!locked, "--offline must not imply --locked");
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
                offline,
                args,
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
                assert!(!force);
                assert!(!locked);
                assert!(!offline);
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
            "--offline",
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
                offline,
                args,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
                assert!(force);
                assert!(locked);
                assert!(offline);
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
                offline,
                filter,
            } => {
                assert!(target.is_none());
                assert!(!release);
                assert!(!verbose);
                assert!(!force);
                assert!(!locked);
                assert!(!offline);
                assert!(filter.is_none());
            }
            other => panic!("expected Test, got {other:?}"),
        }
    }

    #[test]
    fn parse_test_verbose_short() {
        let cli = Cli::try_parse_from(["konvoy", "test", "-v"]).unwrap();
        match cli.command {
            Command::Test { verbose, .. } => assert!(verbose),
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
            "--offline",
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
                offline,
                filter,
            } => {
                assert_eq!(target.as_deref(), Some("linux_x64"));
                assert!(release);
                assert!(verbose);
                assert!(force);
                assert!(locked);
                assert!(offline);
                assert_eq!(filter.as_deref(), Some("MathTest.*"));
            }
            other => panic!("expected Test, got {other:?}"),
        }
    }

    #[test]
    fn parse_clean_defaults() {
        let cli = Cli::try_parse_from(["konvoy", "clean"]).unwrap();
        match cli.command {
            Command::Clean { all } => assert!(!all),
            other => panic!("expected Clean, got {other:?}"),
        }
    }

    #[test]
    fn parse_clean_all() {
        let cli = Cli::try_parse_from(["konvoy", "clean", "--all"]).unwrap();
        match cli.command {
            Command::Clean { all } => assert!(all),
            other => panic!("expected Clean, got {other:?}"),
        }
    }

    #[test]
    fn parse_doctor() {
        let cli = Cli::try_parse_from(["konvoy", "doctor"]).unwrap();
        assert!(matches!(cli.command, Command::Doctor));
    }

    #[test]
    fn parse_update() {
        let cli = Cli::try_parse_from(["konvoy", "update"]).unwrap();
        assert!(matches!(cli.command, Command::Update));
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
    fn error_clean_unknown_flag() {
        let err = Cli::try_parse_from(["konvoy", "clean", "--force"]).unwrap_err();
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
            "generate",
            "update",
            "clean",
            "doctor",
            "check",
            "toolchain",
        ] {
            assert!(help.contains(subcommand));
        }
    }

    // ── Check parsing ────────────────────────────────────────────────

    #[test]
    fn parse_check_defaults_to_human() {
        let cli = Cli::try_parse_from(["konvoy", "check"]).unwrap();
        match cli.command {
            Command::Check { format } => assert!(matches!(format, CheckFormat::Human)),
            other => panic!("expected Check, got {other:?}"),
        }
    }

    #[test]
    fn parse_check_json() {
        let cli = Cli::try_parse_from(["konvoy", "check", "--format", "json"]).unwrap();
        match cli.command {
            Command::Check { format } => assert!(matches!(format, CheckFormat::Json)),
            other => panic!("expected Check, got {other:?}"),
        }
    }

    #[test]
    fn parse_check_rejects_unknown_format() {
        let err = Cli::try_parse_from(["konvoy", "check", "--format", "xml"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn help_flag_on_check() {
        let err = Cli::try_parse_from(["konvoy", "check", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
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
                offline,
            } => {
                assert!(!verbose);
                assert!(config.is_none());
                assert!(!locked);
                assert!(!offline);
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
    fn parse_lint_verbose_short() {
        let cli = Cli::try_parse_from(["konvoy", "lint", "-v"]).unwrap();
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
            "--offline",
        ])
        .unwrap();
        match cli.command {
            Command::Lint {
                verbose,
                config,
                locked,
                offline,
            } => {
                assert!(verbose);
                assert_eq!(config, Some(PathBuf::from("custom.yml")));
                assert!(locked);
                assert!(offline);
            }
            other => panic!("expected Lint, got {other:?}"),
        }
    }

    // ── Generate parsing ─────────────────────────────────────────────

    #[test]
    fn parse_generate_defaults() {
        let cli = Cli::try_parse_from(["konvoy", "generate"]).unwrap();
        match cli.command {
            Command::Generate {
                verbose,
                locked,
                offline,
            } => {
                assert!(!verbose);
                assert!(!locked);
                assert!(!offline);
            }
            other => panic!("expected Generate, got {other:?}"),
        }
    }

    #[test]
    fn parse_generate_verbose_short() {
        let cli = Cli::try_parse_from(["konvoy", "generate", "-v"]).unwrap();
        match cli.command {
            Command::Generate { verbose, .. } => assert!(verbose),
            other => panic!("expected Generate, got {other:?}"),
        }
    }

    #[test]
    fn parse_generate_all_flags() {
        let cli = Cli::try_parse_from(["konvoy", "generate", "--verbose", "--locked", "--offline"])
            .unwrap();
        match cli.command {
            Command::Generate {
                verbose,
                locked,
                offline,
            } => {
                assert!(verbose);
                assert!(locked);
                assert!(offline);
            }
            other => panic!("expected Generate, got {other:?}"),
        }
    }

    #[test]
    fn help_flag_on_generate() {
        let err = Cli::try_parse_from(["konvoy", "generate", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn help_flag_on_update() {
        let err = Cli::try_parse_from(["konvoy", "update", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn error_update_takes_no_args() {
        let err = Cli::try_parse_from(["konvoy", "update", "--force"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownArgument);
    }

    #[test]
    fn help_flag_on_lint() {
        let err = Cli::try_parse_from(["konvoy", "lint", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    // ── clean_project behavior ────────────────────────────────────────

    /// Helper: create a temp project dir with .konvoy/build/ and .konvoy/cache/.
    fn make_clean_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let build_dir = root.join(".konvoy").join("build");
        let cache_dir = root.join(".konvoy").join("cache");
        std::fs::create_dir_all(&build_dir).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(build_dir.join("artifact.exe"), b"binary").unwrap();
        std::fs::write(cache_dir.join("key.json"), b"{}").unwrap();

        tmp
    }

    #[test]
    fn clean_default_removes_only_build_dir() {
        let tmp = make_clean_fixture();
        let root = tmp.path();

        clean_project(root, false).unwrap();

        assert!(
            !root.join(".konvoy").join("build").exists(),
            "build dir should be removed"
        );
        assert!(
            root.join(".konvoy").join("cache").exists(),
            "cache dir should be preserved"
        );
        assert!(
            root.join(".konvoy").exists(),
            ".konvoy dir should be preserved"
        );
    }

    #[test]
    fn clean_all_removes_entire_konvoy_dir() {
        let tmp = make_clean_fixture();
        let root = tmp.path();

        clean_project(root, true).unwrap();

        assert!(
            !root.join(".konvoy").exists(),
            ".konvoy dir should be removed"
        );
    }

    #[test]
    fn clean_default_no_build_dir_is_ok() {
        let tmp = make_clean_fixture();
        let root = tmp.path();

        std::fs::remove_dir_all(root.join(".konvoy").join("build")).unwrap();

        clean_project(root, false).unwrap();

        assert!(
            root.join(".konvoy").join("cache").exists(),
            "cache dir should be preserved"
        );
    }

    #[test]
    fn clean_all_no_konvoy_dir_is_ok() {
        let tmp = make_clean_fixture();
        let root = tmp.path();

        std::fs::remove_dir_all(root.join(".konvoy")).unwrap();

        clean_project(root, true).unwrap();
    }

    // ── Flag → Profile mapping ─────────────────────────────────────

    #[test]
    fn profile_from_flag_false_is_debug() {
        assert_eq!(profile_from_flag(false), konvoy_config::Profile::Debug);
    }

    #[test]
    fn profile_from_flag_true_is_release() {
        assert_eq!(profile_from_flag(true), konvoy_config::Profile::Release);
    }

    // ── build_options constructor ──────────────────────────────────

    #[test]
    fn build_options_passes_fields_through() {
        let opts = build_options(
            Some("linux_x64".to_owned()),
            konvoy_config::Profile::Release,
            true,
            true,
        );
        assert_eq!(opts.target.as_deref(), Some("linux_x64"));
        assert_eq!(opts.profile, konvoy_config::Profile::Release);
        assert!(opts.verbose);
        assert!(opts.force);
    }

    #[test]
    fn build_options_defaults_are_false() {
        let opts = build_options(None, konvoy_config::Profile::Debug, false, false);
        assert!(opts.target.is_none());
        assert_eq!(opts.profile, konvoy_config::Profile::Debug);
        assert!(!opts.verbose);
        assert!(!opts.force);
    }
}
