use anyhow::Result;
use changelogs::config::ChangelogFormat;
use changelogs::{Config, Ecosystem, Package, PublishResult, SkipReason, Workspace};
use std::process::Command;

pub fn run_with_ecosystem(
    dry_run: bool,
    tag: Option<String>,
    ecosystem: Option<Ecosystem>,
) -> Result<()> {
    let workspace = Workspace::load_with_ecosystem(ecosystem)?;
    let config = Config::load(&workspace.changelog_dir)?;

    let all_publishable = workspace.get_publishable_packages()?;
    let packages: Vec<&Package> = all_publishable
        .into_iter()
        .filter(|pkg| !config.ignore.contains(&pkg.name))
        .collect();

    if packages.is_empty() {
        println!("No unpublished packages found");
        return Ok(());
    }

    println!("🚀 Publishing {} package(s)...\n", packages.len());

    let mut published: Vec<&Package> = Vec::new();
    let mut skipped: Vec<&Package> = Vec::new();
    let mut failed: Vec<&Package> = Vec::new();

    for pkg in packages {
        print!("  {} v{} ... ", pkg.name, pkg.version);

        match workspace.publish_package(pkg, dry_run, tag.as_deref()) {
            Ok(PublishResult::Success) => {
                if dry_run {
                    println!("(dry-run)");
                } else {
                    println!("✓");
                }
                published.push(pkg);
            }
            Ok(PublishResult::Skipped(reason)) => {
                match reason {
                    SkipReason::NoToken => println!("⊘ (no token)"),
                    SkipReason::NotPublishable => println!("⊘ (registry publishing disabled)"),
                }
                skipped.push(pkg);
            }
            Ok(PublishResult::Failed) => {
                println!("✗");
                failed.push(pkg);
            }
            Err(e) => {
                println!("✗");
                eprintln!("    {}", e);
                failed.push(pkg);
            }
        }
    }

    println!();

    let mut tag_count = 0;
    if !dry_run {
        let taggable: Vec<&Package> = published.iter().chain(skipped.iter()).copied().collect();
        if !taggable.is_empty() {
            if config.changelog.format == ChangelogFormat::Root {
                // Root format = single product: create one `v{version}` tag
                if let Some(pkg) = workspace.unified_package() {
                    tag_count = create_unified_tag(&workspace, &pkg.version)?;
                }
            } else {
                tag_count = create_git_tags(&workspace, &taggable)?;
            }
        }
    }

    if !failed.is_empty() {
        anyhow::bail!("{} package(s) failed to publish", failed.len());
    }

    if dry_run {
        println!(
            "Dry run complete. {} package(s) would be published.",
            published.len()
        );
    } else if !skipped.is_empty() && published.is_empty() {
        println!(
            "No packages published, but {} git tag(s) created",
            tag_count
        );
    } else {
        println!("Successfully published {} package(s)", published.len());
    }

    Ok(())
}

fn create_unified_tag(workspace: &Workspace, version: &semver::Version) -> Result<usize> {
    let tag = format!("v{}", version);

    let output = Command::new("git")
        .args(["tag", "-a", &tag, "-m", &format!("Release {}", tag)])
        .current_dir(&workspace.root)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run 'git tag': {}", e))?;

    if output.status.success() {
        println!("Created git tag: {}", tag);
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = stderr.trim();
        anyhow::bail!("failed to create git tag {tag}: {error}");
    }

    println!("\nDon't forget to push tags: git push --follow-tags");
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn run_git(root: &std::path::Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .unwrap()
                .success()
        );
    }

    fn package(name: &str, version: &str, path: &str) -> Package {
        Package {
            dependencies: vec![],
            manifest_path: PathBuf::from(path).join("Cargo.toml"),
            name: name.to_string(),
            path: PathBuf::from(path),
            version: version.parse().unwrap(),
        }
    }

    fn workspace(root: &str) -> Workspace {
        let root = PathBuf::from(root);
        Workspace {
            changelog_dir: root.join(".changelog"),
            ecosystem: Ecosystem::Rust,
            packages: vec![],
            root,
        }
    }

    #[test]
    fn unified_tag_uses_root_package() {
        let mut workspace = workspace("/repo");
        let helper = package("helper", "9.0.0", "/repo/crates/helper");
        let root = package("product", "1.2.3", "/repo");
        workspace.packages = vec![helper, root];

        let selected = workspace.unified_package().unwrap();

        assert_eq!(selected.name, "product");
        assert_eq!(selected.version, Version::new(1, 2, 3));
    }

    #[test]
    fn unified_tag_has_deterministic_virtual_workspace_fallback() {
        let mut workspace = workspace("/repo");
        let second = package("second", "1.2.3", "/repo/crates/z-second");
        let first = package("first", "1.2.3", "/repo/crates/a-first");
        workspace.packages = vec![second, first];

        let selected = workspace.unified_package().unwrap();

        assert_eq!(selected.name, "first");
    }

    #[test]
    fn unified_tag_reports_existing_tag_failure() {
        let dir = TempDir::new().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        run_git(dir.path(), &["commit", "--allow-empty", "-qm", "initial"]);
        let workspace = workspace(dir.path().to_str().unwrap());
        let version = Version::new(1, 2, 3);

        assert_eq!(create_unified_tag(&workspace, &version).unwrap(), 1);
        assert!(create_unified_tag(&workspace, &version).is_err());
    }
}

fn create_git_tags(workspace: &Workspace, packages: &[&Package]) -> Result<usize> {
    let mut created = 0;
    for pkg in packages {
        let tag = workspace.tag_name(pkg);

        let output = Command::new("git")
            .args(["tag", "-a", &tag, "-m", &format!("Release {}", tag)])
            .current_dir(&workspace.root)
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run 'git tag': {}", e))?;

        if output.status.success() {
            println!("Created git tag: {}", tag);
            created += 1;
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let error = stderr.trim();
            anyhow::bail!("failed to create git tag {tag}: {error}");
        }
    }

    println!("\nDon't forget to push tags: git push --follow-tags");
    Ok(created)
}
