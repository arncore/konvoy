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

    match cli.command {
        Command::Init { name } => {
            let name = name.as_deref().unwrap_or("my-project");
            println!("konvoy init: creating project '{name}'");
        }
        Command::Build { target, release, verbose } => {
            let profile = if release { "release" } else { "debug" };
            let target_str = target.as_deref().unwrap_or("host");
            println!("konvoy build: target={target_str} profile={profile} verbose={verbose}");
        }
        Command::Run { target, release, args } => {
            let profile = if release { "release" } else { "debug" };
            let target_str = target.as_deref().unwrap_or("host");
            println!("konvoy run: target={target_str} profile={profile} args={args:?}");
        }
        Command::Test { target, release, verbose } => {
            let profile = if release { "release" } else { "debug" };
            let target_str = target.as_deref().unwrap_or("host");
            println!("konvoy test: target={target_str} profile={profile} verbose={verbose}");
        }
        Command::Clean => {
            println!("konvoy clean");
        }
        Command::Doctor => {
            println!("konvoy doctor: checking environment...");
        }
    }
}
