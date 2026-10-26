use anyhow::Result;
use changelogs::config::ChangelogFormat;
use changelogs::graph::DependencyGraph;
use changelogs::output::{self, FailedPackage, PublishOutput, PublishedPackage, SkippedPackage};
use changelogs::{Config, Ecosystem, Package, PublishResult, SkipReason, Workspace};
use std::collections::{HashMap, HashSet};
use std::process::Command;

pub fn run_with_ecosystem(
    dry_run: bool,
    tag: Option<String>,
    json: bool,
    ecosystem: Option<Ecosystem>,
) -> Result<()> {
    let mut report = PublishOutput::default();
    let result = run(dry_run, tag, json, ecosystem, &mut report);
    // Print the report even when publishing failed, so callers see which
    // packages made it and which tags to push.
    if json {
        output::print_json(&report)?;
    }
    result
}

fn run(
    dry_run: bool,
    tag: Option<String>,
    json: bool,
    ecosystem: Option<Ecosystem>,
    report: &mut PublishOutput,
) -> Result<()> {
    let workspace = Workspace::load_with_ecosystem(ecosystem)?;
    let config = Config::load(&workspace.changelog_dir)?;

    let all_publishable = workspace.get_publishable_packages()?;
    let packages: Vec<&Package> = all_publishable
        .into_iter()
        .filter(|pkg| !config.ignore.contains(&pkg.name))
        .collect();

    if packages.is_empty() {
        say!(json, "No unpublished packages found");
        return Ok(());
    }

    let graph = DependencyGraph::from_workspace(&workspace);
    let packages = publish_order(&graph, packages);

    say!(json, "🚀 Publishing {} package(s)...\n", packages.len());

    let unified_version = workspace.unified_package().map(|pkg| pkg.version.clone());
    let release_tag = |pkg: &Package| match (&config.changelog.format, &unified_version) {
        (ChangelogFormat::Root, Some(version)) => format!("v{}", version),
        _ => workspace.tag_name(pkg),
    };

    let mut published: Vec<&Package> = Vec::new();
    let mut skipped: Vec<&Package> = Vec::new();
    let mut failed: Vec<&Package> = Vec::new();
    // Dependent package name -> the failed dependency it waits on.
    let mut blocked: HashMap<String, String> = HashMap::new();

    for pkg in packages {
        if json {
            eprint!("  {} v{} ... ", pkg.name, pkg.version);
        } else {
            print!("  {} v{} ... ", pkg.name, pkg.version);
        }
        let mut fail = |error: String| {
            report.failed.push(FailedPackage {
                name: pkg.name.clone(),
                version: pkg.version.to_string(),
                error,
            });
            failed.push(pkg);
        };

        if let Some(dependency) = blocked.get(&pkg.name) {
            say!(json, "⊘ (skipped: dependency {} failed)", dependency);
            fail(format!("dependency {} failed", dependency));
            continue;
        }

        match workspace.publish_package(pkg, dry_run, tag.as_deref()) {
            Ok(PublishResult::Success) => {
                if dry_run {
                    say!(json, "(dry-run)");
                } else {
                    say!(json, "✓");
                }
                report.published.push(PublishedPackage {
                    name: pkg.name.clone(),
                    version: pkg.version.to_string(),
                    tag: release_tag(pkg),
                });
                published.push(pkg);
            }
            Ok(PublishResult::Skipped(reason)) => {
                let reason = match reason {
                    SkipReason::NoToken => {
                        say!(json, "⊘ (no token)");
                        "no_token"
                    }
                    SkipReason::NotPublishable => {
                        say!(json, "⊘ (registry publishing disabled)");
                        "not_publishable"
                    }
                };
                report.skipped.push(SkippedPackage {
                    name: pkg.name.clone(),
                    version: pkg.version.to_string(),
                    tag: release_tag(pkg),
                    reason: reason.to_string(),
                });
                skipped.push(pkg);
            }
            Ok(PublishResult::Failed) => {
                say!(json, "✗");
                fail("publish failed".to_string());
                block_dependents(&graph, &pkg.name, &mut blocked);
            }
            Err(e) => {
                say!(json, "✗");
                eprintln!("    {}", e);
                fail(e.to_string());
                block_dependents(&graph, &pkg.name, &mut blocked);
            }
        }
    }

    say!(json);

    if !dry_run {
        let taggable: Vec<&Package> = published.iter().chain(skipped.iter()).copied().collect();
        if !taggable.is_empty() {
            if config.changelog.format == ChangelogFormat::Root {
                // Root format = single product: create one `v{version}` tag, only
                // once every package is out, so a rerun retries the failed ones.
                if !failed.is_empty() {
                    say!(
                        json,
                        "Not tagging the release: {} package(s) failed",
                        failed.len()
                    );
                } else if let Some(pkg) = workspace.unified_package() {
                    create_unified_tag(&workspace, &pkg.version, json, &mut report.tags_created)?;
                }
            } else {
                create_git_tags(&workspace, &taggable, json, &mut report.tags_created)?;
            }
        }
    }

    if !failed.is_empty() {
        anyhow::bail!("{} package(s) failed to publish", failed.len());
    }

    if dry_run {
        say!(
            json,
            "Dry run complete. {} package(s) would be published.",
            published.len()
        );
    } else if !skipped.is_empty() && published.is_empty() {
        say!(
            json,
            "No packages published, but {} git tag(s) created",
            report.tags_created.len()
        );
    } else {
        say!(
            json,
            "Successfully published {} package(s)",
            published.len()
        );
    }

    Ok(())
}

/// Orders packages so each one's workspace dependencies are published first,
/// keeping the input order otherwise.
fn publish_order<'a>(graph: &DependencyGraph, packages: Vec<&'a Package>) -> Vec<&'a Package> {
    fn visit<'a>(
        graph: &DependencyGraph,
        name: &str,
        by_name: &HashMap<&str, &'a Package>,
        visited: &mut HashSet<String>,
        ordered: &mut Vec<&'a Package>,
    ) {
        if !visited.insert(name.to_string()) {
            return;
        }
        let mut dependencies = graph.dependencies(name);
        dependencies.sort();
        for dependency in &dependencies {
            visit(graph, dependency, by_name, visited, ordered);
        }
        if let Some(pkg) = by_name.get(name) {
            ordered.push(pkg);
        }
    }

    let by_name: HashMap<&str, &Package> = packages
        .iter()
        .map(|pkg| (pkg.name.as_str(), *pkg))
        .collect();
    let mut visited = HashSet::new();
    let mut ordered = Vec::with_capacity(packages.len());
    for pkg in &packages {
        visit(graph, &pkg.name, &by_name, &mut visited, &mut ordered);
    }
    ordered
}

fn block_dependents(graph: &DependencyGraph, failed: &str, blocked: &mut HashMap<String, String>) {
    for dependent in graph.all_dependents(failed) {
        blocked
            .entry(dependent)
            .or_insert_with(|| failed.to_string());
    }
}

/// Creates the root format's `v{version}` tag, adding it to `created`.
fn create_unified_tag(
    workspace: &Workspace,
    version: &semver::Version,
    json: bool,
    created: &mut Vec<String>,
) -> Result<()> {
    let tag = format!("v{}", version);
    if workspace.tag_exists(&tag) {
        say!(json, "Git tag {} already exists, skipping", tag);
        return Ok(());
    }

    let output = Command::new("git")
        .args(["tag", "-a", &tag, "-m", &format!("Release {}", tag)])
        .current_dir(&workspace.root)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run 'git tag': {}", e))?;

    if output.status.success() {
        say!(json, "Created git tag: {}", tag);
        created.push(tag);
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = stderr.trim();
        anyhow::bail!("failed to create git tag {tag}: {error}");
    }

    say!(json, "\nDon't forget to push tags: git push --follow-tags");
    Ok(())
}

/// Creates each package's release tag, adding the new ones to `created`.
fn create_git_tags(
    workspace: &Workspace,
    packages: &[&Package],
    json: bool,
    created: &mut Vec<String>,
) -> Result<()> {
    for pkg in packages {
        let tag = workspace.tag_name(pkg);
        if workspace.tag_exists(&tag) {
            say!(json, "Git tag {} already exists, skipping", tag);
            continue;
        }

        let output = Command::new("git")
            .args(["tag", "-a", &tag, "-m", &format!("Release {}", tag)])
            .current_dir(&workspace.root)
            .output()
            .map_err(|e| anyhow::anyhow!("failed to run 'git tag': {}", e))?;

        if output.status.success() {
            say!(json, "Created git tag: {}", tag);
            created.push(tag);
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let error = stderr.trim();
            anyhow::bail!("failed to create git tag {tag}: {error}");
        }
    }

    say!(json, "\nDon't forget to push tags: git push --follow-tags");
    Ok(())
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
    fn publish_order_puts_dependencies_first() {
        let mut workspace = workspace("/repo");
        let mut app = package("app", "1.0.0", "/repo/crates/a-app");
        app.dependencies = vec!["lib".to_string()];
        let mut cli = package("cli", "1.0.0", "/repo/crates/b-cli");
        cli.dependencies = vec!["app".to_string(), "lib".to_string()];
        let lib = package("lib", "1.0.0", "/repo/crates/z-lib");
        workspace.packages = vec![app, cli, lib];
        let graph = DependencyGraph::from_workspace(&workspace);

        let ordered = publish_order(&graph, workspace.packages.iter().collect());

        let names: Vec<&str> = ordered.iter().map(|pkg| pkg.name.as_str()).collect();
        assert_eq!(names, vec!["lib", "app", "cli"]);
    }

    #[test]
    fn failed_package_blocks_transitive_dependents() {
        let mut workspace = workspace("/repo");
        let mut app = package("app", "1.0.0", "/repo/crates/a-app");
        app.dependencies = vec!["lib".to_string()];
        let mut cli = package("cli", "1.0.0", "/repo/crates/b-cli");
        cli.dependencies = vec!["app".to_string()];
        let other = package("other", "1.0.0", "/repo/crates/other");
        let lib = package("lib", "1.0.0", "/repo/crates/z-lib");
        workspace.packages = vec![app, cli, other, lib];
        let graph = DependencyGraph::from_workspace(&workspace);
        let mut blocked = HashMap::new();

        block_dependents(&graph, "lib", &mut blocked);

        assert_eq!(blocked.get("app").map(String::as_str), Some("lib"));
        assert_eq!(blocked.get("cli").map(String::as_str), Some("lib"));
        assert!(!blocked.contains_key("other"));
    }

    #[test]
    fn unified_tag_skips_existing_tag() {
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

        let mut created = Vec::new();
        create_unified_tag(&workspace, &version, false, &mut created).unwrap();
        create_unified_tag(&workspace, &version, false, &mut created).unwrap();
        assert_eq!(created, vec!["v1.2.3"]);
    }
}
