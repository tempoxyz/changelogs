use anyhow::{Context, Result};
use changelogs::Ecosystem;
use changelogs::changelog_entry;
use changelogs::changelog_writer;
use changelogs::config::Config;
use changelogs::error::Error;
use changelogs::plan;
use changelogs::workspace::Workspace;
use console::style;
use semver::Version;
use std::collections::HashMap;

pub fn run(dry_run: bool, ecosystem: Option<Ecosystem>) -> Result<()> {
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

    if release_plan.releases.is_empty() {
        println!("{} No packages to release", style("ℹ").blue().bold());
        return Ok(());
    }

    if !release_plan.warnings.is_empty() {
        for warning in &release_plan.warnings {
            println!(
                "  {} {}",
                style("!").yellow().bold(),
                style(warning).yellow()
            );
        }
        println!();
    }

    println!("{} Updating versions...\n", style("→").blue().bold());

    println!("{} Release plan:\n", style("→").blue().bold());

    for release in &release_plan.releases {
        println!(
            "  {} {} {} → {}",
            style("✓").green(),
            style(&release.name).cyan(),
            style(&release.old_version.to_string()).dim(),
            style(&release.new_version.to_string()).green()
        );
    }

    if dry_run {
        println!(
            "\n{} {} package(s) would be updated (dry run — no files changed)",
            style("ℹ").blue().bold(),
            release_plan.releases.len()
        );
        return Ok(());
    }

    println!("\n{} Updating versions...\n", style("→").blue().bold());

    let mut version_updates: HashMap<String, Version> = HashMap::new();
    for release in &release_plan.releases {
        workspace.update_version(&release.name, &release.new_version)?;
        version_updates.insert(release.name.clone(), release.new_version.clone());
    }
    workspace.update_dependency_versions(&version_updates)?;

    println!("{} Updating changelogs...\n", style("→").blue().bold());

    changelog_writer::write_changelogs(
        &workspace,
        &release_plan.releases,
        &changelogs,
        config.changelog.format,
    )?;

    for release in &release_plan.releases {
        println!(
            "  {} Updated CHANGELOG.md for {}",
            style("✓").green(),
            style(&release.name).cyan()
        );
    }

    println!("\n{} Removing changelogs...\n", style("→").blue().bold());

    for cs in &changelogs {
        changelog_entry::delete(&changelog_dir, &cs.id)?;
        println!(
            "  {} Deleted {}",
            style("✓").green(),
            style(format!("{}.md", cs.id)).dim()
        );
    }

    println!(
        "\n{} {} package(s) updated",
        style("✓").green().bold(),
        release_plan.releases.len()
    );

    Ok(())
}
