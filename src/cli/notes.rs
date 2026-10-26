use anyhow::{Context, Result};
use changelogs::config::{ChangelogFormat, Config};
use changelogs::{Ecosystem, Workspace, changelog_writer};

/// Prints the CHANGELOG.md section of release `tag`, without its heading, such
/// as for GitHub release notes. `tag` must be the release tag of a package's
/// current version, as `publish` creates it.
pub fn run(tag: &str, ecosystem: Option<Ecosystem>) -> Result<()> {
    let workspace = Workspace::load_with_ecosystem(ecosystem)?;
    let config = Config::load(&workspace.changelog_dir)?;

    let (package, path) = match config.changelog.format {
        ChangelogFormat::Root => {
            let package = workspace
                .unified_package()
                .filter(|pkg| format!("v{}", pkg.version) == tag);
            (package, workspace.root.join("CHANGELOG.md"))
        }
        ChangelogFormat::PerCrate => {
            let package = workspace
                .packages
                .iter()
                .find(|pkg| workspace.tag_name(pkg) == tag);
            let path = package.map_or_else(|| workspace.root.clone(), |pkg| pkg.path.clone());
            (package, path.join("CHANGELOG.md"))
        }
    };
    let package =
        package.with_context(|| format!("no package's current version is released as {tag}"))?;

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let notes = changelog_writer::find_section(&content, &package.name, &package.version)
        .with_context(|| format!("{} has no section for {tag}", path.display()))?;
    println!("{notes}");
    Ok(())
}
