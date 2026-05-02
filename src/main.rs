use anyhow::Result;
use changelogs::Ecosystem;
use clap::{Parser, Subcommand};

mod cli;

#[derive(Parser)]
#[command(name = "changelogs")]
#[command(about = "Manage versioning and changelogs for workspaces")]
#[command(version)]
struct Cli {
    /// Ecosystem to use (rust, python, go). Auto-detected if not specified.
    #[arg(short = 'e', long, global = true)]
    ecosystem: Option<Ecosystem>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new changelog
    Add {
        /// Create an empty changelog (no packages)
        #[arg(long)]
        empty: bool,

        /// Use AI to generate the changelog from git diff
        #[arg(short = 'a', long)]
        ai: Option<String>,

        /// Custom instructions for AI generation
        #[arg(short = 'i', long)]
        instructions: Option<String>,

        /// Base ref to diff against (e.g. origin/main)
        #[arg(short = 'r', long = "ref")]
        base_ref: Option<String>,
    },

    /// Validate workspace configuration
    Doctor,

    /// Initialize changelogs in this workspace
    Init,

    /// Publish unpublished packages to crates.io
    Publish {
        /// Perform a dry run without actually publishing
        #[arg(long)]
        dry_run: bool,

        /// Registry to publish to
        #[arg(long)]
        tag: Option<String>,
    },

    /// Show pending changelogs and releases
    Status {
        /// Show detailed changelog contents
        #[arg(long)]
        verbose: bool,
    },

    /// Update changelogs to the latest version
    Up,

    /// Apply version bumps and update changelogs
    Version {
        /// Compute and display the release plan without writing any files
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add {
            empty,
            ai,
            instructions,
            base_ref,
        } => cli::add::run(empty, ai, instructions, base_ref, cli.ecosystem)?,
        Commands::Doctor => cli::doctor::run(cli.ecosystem)?,
        Commands::Init => cli::init::run(cli.ecosystem)?,
        Commands::Publish { dry_run, tag } => {
            cli::publish::run_with_ecosystem(dry_run, tag, cli.ecosystem)?
        }
        Commands::Status { verbose } => cli::status::run(verbose, cli.ecosystem)?,
        Commands::Up => cli::up::run()?,
        Commands::Version { dry_run } => cli::version::run(dry_run, cli.ecosystem)?,
    }

    Ok(())
}
