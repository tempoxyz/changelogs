use anyhow::{Context, Result};
use changelogs::changelog_entry;
use changelogs::config::Config;
use changelogs::error::Error;
use changelogs::plan;
use changelogs::workspace::Workspace;
use changelogs::{BumpType, Ecosystem};
use console::style;

pub fn run(verbose: bool, ecosystem: Option<Ecosystem>) -> Result<()> {
    let workspace = Workspace::discover_with_ecosystem(ecosystem).context(
        "could not detect workspace — specify ecosystem with: changelogs --ecosystem <rust|python|go>",
    )?;

    if !workspace.is_initialized() {
        return Err(Error::NotInitialized.into());
    }

    let changelog_dir = workspace.changelog_dir();
    let changelogs = changelog_entry::read_all(&changelog_dir)?;

    if changelogs.is_empty() {
        println!("{} No changelogs found", style("ℹ").blue().bold());
        return Ok(());
    }

    let config = Config::load(&changelog_dir)?;
    let release_plan = plan::assemble(&workspace, changelogs.clone(), &config);

    println!(
        "{} {} changelog(s) found\n",
        style("ℹ").blue().bold(),
        changelogs.len()
    );

    if verbose {
        println!("{}", style("Changelogs:").bold().underlined());
        for cs in &changelogs {
            println!("\n  {} {}", style("•").dim(), style(&cs.id).cyan());
            for release in &cs.releases {
                println!(
                    "    {} {} ({})",
                    style("→").dim(),
                    release.package,
                    style(release.bump.to_string()).yellow()
                );
            }
            if !cs.summary.is_empty() {
                println!("    {}", style(&cs.summary).dim());
            }
        }
        println!();
    }

    if release_plan.releases.is_empty() {
        println!("{} No packages will be released", style("ℹ").blue().bold());
        return Ok(());
    }

    println!("{}", style("Releases:").bold().underlined());
    for release in &release_plan.releases {
        let bump_style = match release.bump {
            BumpType::Major => style(release.bump.to_string()).red().bold(),
            BumpType::Minor => style(release.bump.to_string()).yellow(),
            BumpType::Patch => style(release.bump.to_string()).dim(),
        };

        println!(
            "  {} {} {} → {} ({})",
            style("•").dim(),
            style(&release.name).cyan(),
            style(&release.old_version.to_string()).dim(),
            style(&release.new_version.to_string()).green(),
            bump_style
        );
    }

    if !release_plan.warnings.is_empty() {
        println!();
        for warning in &release_plan.warnings {
            println!(
                "  {} {}",
                style("!").yellow().bold(),
                style(warning).yellow()
            );
        }
    }

    Ok(())
}
