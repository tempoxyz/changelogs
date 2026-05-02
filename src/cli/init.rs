use anyhow::{Context, Result};
use changelogs::Ecosystem;
use changelogs::config::Config;
use changelogs::error::Error;
use changelogs::workspace::Workspace;
use console::style;

pub fn run(ecosystem: Option<Ecosystem>) -> Result<()> {
    let workspace = Workspace::discover_with_ecosystem(ecosystem)
        .context("could not detect workspace — specify ecosystem with: changelogs --ecosystem <rust|python|go> init")?;

    if workspace.is_initialized() {
        return Err(Error::AlreadyInitialized.into());
    }

    let changelog_dir = workspace.changelog_dir();
    std::fs::create_dir_all(&changelog_dir)?;

    std::fs::write(changelog_dir.join("config.toml"), Config::default_toml())?;

    std::fs::write(
        changelog_dir.join("README.md"),
        r#"# Changelogs

This folder contains changelog files that describe changes to be released.

## Adding a changelog

Run `changelogs add` to create a new changelog file.

## File format

Changelog files are markdown with YAML frontmatter:

```markdown
---
package-name: minor
other-package: patch
---

Description of the changes made.
```

## Releasing

Run `changelogs version` to apply version bumps and generate changelogs.
"#,
    )?;

    println!(
        "{} Initialized changelogs in {}",
        style("✓").green().bold(),
        changelog_dir.display()
    );

    println!("\nNext steps:");
    println!(
        "  1. Run {} to create your first changelog",
        style("changelogs add").cyan()
    );
    println!("  2. Commit the changelog file with your PR");
    println!(
        "  3. Run {} to apply versions",
        style("changelogs version").cyan()
    );

    Ok(())
}
