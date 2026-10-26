use anyhow::{Context, Result};
use changelogs::Ecosystem;
use changelogs::changelog_entry;
use changelogs::changelog_writer::{self, Section};
use changelogs::config::ChangelogFormat;
use changelogs::error::Error;
use changelogs::output::{self, VersionOutput, VersionRelease};
use changelogs::plan::{self, ReleasePlan};
use changelogs::workspace::Workspace;
use console::style;
use semver::Version;
use std::collections::HashMap;

pub fn run(
    dry_run: bool,
    prerelease: Option<String>,
    json: bool,
    ecosystem: Option<Ecosystem>,
) -> Result<()> {
    let prerelease = prerelease
        .map(plan::PrereleasePrefix::new)
        .transpose()
        .map_err(anyhow::Error::msg)?;

    let workspace = Workspace::discover_with_ecosystem(ecosystem).context(
        "could not detect workspace — specify ecosystem with: changelogs --ecosystem <rust|python|go|swift>",
    )?;

    if !workspace.is_initialized() {
        return Err(Error::NotInitialized.into());
    }

    let changelog_dir = workspace.changelog_dir();
    let changelogs = changelog_entry::read_all(&changelog_dir)?;
    let config = &workspace.config;
    let unknown = plan::unknown_package_refs(&workspace, &changelogs, config);
    if !unknown.is_empty() {
        let details = unknown
            .iter()
            .map(|(id, package)| {
                format!(
                    "  {} references unknown package '{}'",
                    changelog_dir.join(format!("{}.md", id)).display(),
                    package
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::bail!(
            "changelog entries reference packages that are not in the workspace (fix the entry or add the package to `ignore` in config):\n{}",
            details
        );
    }

    let has_changelogs = !changelogs.is_empty();
    let release_plan = if has_changelogs {
        plan::assemble_with_prerelease(&workspace, changelogs.clone(), config, prerelease.as_ref())
    } else if prerelease.is_none() {
        plan::assemble_stable_promotions(&workspace, config)
    } else {
        say!(json, "{} No changelogs found", style("ℹ").blue().bold());
        return print_version_json(json, &workspace, config.changelog.format, None, &[]);
    };

    if release_plan.releases.is_empty() {
        if has_changelogs {
            say!(json, "{} No packages to release", style("ℹ").blue().bold());
        } else {
            say!(json, "{} No changelogs found", style("ℹ").blue().bold());
        }
        return print_version_json(json, &workspace, config.changelog.format, None, &[]);
    }

    let sections = if has_changelogs {
        changelog_writer::render_sections(
            &workspace,
            &release_plan.releases,
            &changelogs,
            config.changelog.format,
        )
    } else {
        Vec::new()
    };

    if !release_plan.warnings.is_empty() {
        for warning in &release_plan.warnings {
            say!(
                json,
                "  {} {}",
                style("!").yellow().bold(),
                style(warning).yellow()
            );
        }
        say!(json);
    }

    say!(json, "{} Updating versions...\n", style("→").blue().bold());

    say!(json, "{} Release plan:\n", style("→").blue().bold());

    for release in &release_plan.releases {
        say!(
            json,
            "  {} {} {} → {}",
            style("✓").green(),
            style(&release.name).cyan(),
            style(&release.old_version.to_string()).dim(),
            style(&release.new_version.to_string()).green()
        );
    }

    if dry_run {
        say!(
            json,
            "\n{} {} package(s) would be updated (dry run — no files changed)",
            style("ℹ").blue().bold(),
            release_plan.releases.len()
        );
        return print_version_json(
            json,
            &workspace,
            config.changelog.format,
            Some(&release_plan),
            &sections,
        );
    }

    say!(
        json,
        "\n{} Updating versions...\n",
        style("→").blue().bold()
    );

    let mut version_updates: HashMap<String, Version> = HashMap::new();
    for release in &release_plan.releases {
        workspace.update_version(&release.name, &release.new_version)?;
        version_updates.insert(release.name.clone(), release.new_version.clone());
    }
    workspace.update_dependency_versions(&version_updates)?;

    if !has_changelogs {
        say!(
            json,
            "\n{} {} package(s) updated",
            style("✓").green().bold(),
            release_plan.releases.len()
        );
        return print_version_json(
            json,
            &workspace,
            config.changelog.format,
            Some(&release_plan),
            &sections,
        );
    }

    say!(
        json,
        "{} Updating changelogs...\n",
        style("→").blue().bold()
    );

    changelog_writer::write_sections(&sections, config.changelog.format)?;

    for release in &release_plan.releases {
        say!(
            json,
            "  {} Updated CHANGELOG.md for {}",
            style("✓").green(),
            style(&release.name).cyan()
        );
    }

    say!(
        json,
        "\n{} Removing changelogs...\n",
        style("→").blue().bold()
    );

    for cs in &changelogs {
        changelog_entry::delete(&changelog_dir, &cs.id)?;
        say!(
            json,
            "  {} Deleted {}",
            style("✓").green(),
            style(format!("{}.md", cs.id)).dim()
        );
    }

    say!(
        json,
        "\n{} {} package(s) updated",
        style("✓").green().bold(),
        release_plan.releases.len()
    );

    print_version_json(
        json,
        &workspace,
        config.changelog.format,
        Some(&release_plan),
        &sections,
    )
}

/// With `json`, prints the release plan and the changelog sections it adds
/// as a [`VersionOutput`].
fn print_version_json(
    json: bool,
    workspace: &Workspace,
    format: ChangelogFormat,
    release_plan: Option<&ReleasePlan>,
    sections: &[Section],
) -> Result<()> {
    if !json {
        return Ok(());
    }
    let releases = release_plan.map_or(&[][..], |plan| &plan.releases[..]);
    let relative = |path: &std::path::Path| {
        path.strip_prefix(&workspace.root)
            .unwrap_or(path)
            .display()
            .to_string()
    };

    let output = VersionOutput {
        format,
        releases: releases
            .iter()
            .map(|release| {
                let section = sections
                    .iter()
                    .find(|section| section.packages.contains(&release.name));
                let changelog_path = match (section, format) {
                    (Some(section), _) => section.path.clone(),
                    (None, ChangelogFormat::Root) => workspace.root.join("CHANGELOG.md"),
                    (None, ChangelogFormat::PerCrate) => workspace
                        .get_package(&release.name)
                        .map_or_else(|| workspace.root.clone(), |pkg| pkg.path.clone())
                        .join("CHANGELOG.md"),
                };
                VersionRelease {
                    name: release.name.clone(),
                    old_version: release.old_version.to_string(),
                    new_version: release.new_version.to_string(),
                    bump: release.bump,
                    is_promotion: !release.old_version.pre.is_empty()
                        && release.new_version.pre.is_empty(),
                    changelog_path: relative(&changelog_path),
                    notes: section.map(|s| s.text.clone()).unwrap_or_default(),
                }
            })
            .collect(),
        unified_version: (format == ChangelogFormat::Root)
            .then(|| {
                let unified = workspace.unified_package().map(|pkg| pkg.name.as_str());
                releases
                    .iter()
                    .find(|release| Some(release.name.as_str()) == unified)
                    .or(releases.first())
                    .map(|release| release.new_version.to_string())
            })
            .flatten(),
    };
    output::print_json(&output)?;
    Ok(())
}
