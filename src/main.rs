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

    /// Publish unpublished packages to their registry (crates.io, PyPI) and tag the releases
    Publish {
        /// Perform a dry run without actually publishing
        #[arg(long)]
        dry_run: bool,

        /// Registry to publish to instead of crates.io or PyPI: a Cargo registry name, or a PyPI
        /// repository name or URL
        #[arg(long, alias = "tag")]
        registry: Option<String>,

        /// Print the results as JSON on stdout (human output goes to stderr)
        #[arg(long)]
        json: bool,
    },

    /// Print the changelog notes of a release tag, such as v1.2.0 or name@1.2.0
    Notes {
        /// The release tag of a package's current version
        tag: String,
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

        /// Add or increment a prerelease identifier, such as rc for rc1, rc2
        #[arg(long)]
        prerelease: Option<String>,

        /// Print the release plan as JSON on stdout (human output goes to stderr)
        #[arg(long)]
        json: bool,
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
        Commands::Notes { tag } => cli::notes::run(&tag, cli.ecosystem)?,
        Commands::Publish {
            dry_run,
            registry,
            json,
        } => cli::publish::run_with_ecosystem(dry_run, registry, json, cli.ecosystem)?,
        Commands::Status { verbose } => cli::status::run(verbose, cli.ecosystem)?,
        Commands::Up => cli::up::run()?,
        Commands::Version {
            dry_run,
            prerelease,
            json,
        } => cli::version::run(dry_run, prerelease, json, cli.ecosystem)?,
    }

    Ok(())
}
