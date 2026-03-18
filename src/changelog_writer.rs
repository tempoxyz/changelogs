use crate::BumpType;
use crate::changelog_entry::{self, Changelog};
use crate::config::ChangelogFormat;
use crate::error::Result;
use crate::plan::PackageRelease;
use crate::workspace::Workspace;
use chrono::Utc;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::process::Command;

fn get_github_url() -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if url.starts_with("git@github.com:") {
        let repo = url
            .strip_prefix("git@github.com:")?
            .strip_suffix(".git")
            .unwrap_or(&url);
        Some(format!("https://github.com/{}", repo))
    } else if url.starts_with("https://github.com/") {
        Some(url.strip_suffix(".git").unwrap_or(&url).to_string())
    } else {
        None
    }
}

struct ChangeWithMeta {
    summary: String,
    link: Option<(String, String)>, // (url, display_text)
    authors: Vec<String>,
}

pub fn generate_entry(
    release: &PackageRelease,
    changelogs: &[Changelog],
    changelog_dir: &Path,
) -> String {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    generate_entry_with_date(release, changelogs, changelog_dir, &date)
}

pub fn generate_entry_with_date(
    release: &PackageRelease,
    changelogs: &[Changelog],
    changelog_dir: &Path,
    date: &str,
) -> String {
    let mut entry = format!("## {} ({})\n\n", release.new_version, date);

    let github_url = get_github_url();

    let mut major_changes = Vec::new();
    let mut minor_changes = Vec::new();
    let mut patch_changes = Vec::new();

    for changelog in changelogs {
        if !release.changelog_ids.contains(&changelog.id) {
            continue;
        }

        for rel in &changelog.releases {
            if rel.package != release.name {
                continue;
            }

            let summary = changelog.summary.trim().to_string();

            let (link_info, authors) = github_url
                .as_ref()
                .and_then(|base| {
                    let info = changelog_entry::get_commit_info(changelog_dir, &changelog.id)?;

                    let link_info = if let Some(pr) = info.pr_number {
                        Some((format!("{}/pull/{}", base, pr), format!("#{}", pr)))
                    } else {
                        let short_sha = &info.commit_sha[..7.min(info.commit_sha.len())];
                        Some((
                            format!("{}/commit/{}", base, short_sha),
                            short_sha.to_string(),
                        ))
                    };

                    Some((link_info, info.authors))
                })
                .unwrap_or((None, Vec::new()));

            let change = ChangeWithMeta {
                summary,
                link: link_info,
                authors,
            };
            match rel.bump {
                BumpType::Major => major_changes.push(change),
                BumpType::Minor => minor_changes.push(change),
                BumpType::Patch => patch_changes.push(change),
            }
        }
    }

    if !major_changes.is_empty() {
        entry.push_str("### Major Changes\n\n");
        for change in major_changes {
            write_change_lines(&mut entry, &change);
        }
        entry.push('\n');
    }

    if !minor_changes.is_empty() {
        entry.push_str("### Minor Changes\n\n");
        for change in minor_changes {
            write_change_lines(&mut entry, &change);
        }
        entry.push('\n');
    }

    if !patch_changes.is_empty() {
        entry.push_str("### Patch Changes\n\n");
        for change in patch_changes {
            write_change_lines(&mut entry, &change);
        }
        entry.push('\n');
    }

    entry
}

fn write_change_lines(entry: &mut String, change: &ChangeWithMeta) {
    let mut suffix_parts = Vec::new();

    if !change.authors.is_empty() {
        let authors_str = change
            .authors
            .iter()
            .map(|a| format!("@{}", a.replace(' ', "")))
            .collect::<Vec<_>>()
            .join(", ");
        suffix_parts.push(format!("by {}", authors_str));
    }

    if let Some((ref url, ref display)) = change.link {
        suffix_parts.push(format!("[{}]({})", display, url));
    }

    let suffix = if suffix_parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", suffix_parts.join(", "))
    };

    let lines: Vec<&str> = change.summary.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let is_last = i == lines.len() - 1;
        let line_suffix = if is_last { &suffix } else { "" };

        if line.starts_with('-') || line.starts_with('*') {
            entry.push_str(&format!("{}{}\n", line, line_suffix));
        } else if !line.is_empty() {
            entry.push_str(&format!("- {}{}\n", line, line_suffix));
        }
    }
}

pub fn update_changelog(path: &Path, new_entry: &str) -> Result<()> {
    let existing = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };

    let new_content = if existing.starts_with("# Changelog") {
        let rest = existing.strip_prefix("# Changelog").unwrap_or(&existing);
        let rest = rest.trim_start_matches('\n');
        format!("# Changelog\n\n{}{}", new_entry, rest)
    } else if existing.is_empty() {
        format!("# Changelog\n\n{}", new_entry)
    } else {
        format!("# Changelog\n\n{}{}", new_entry, existing)
    };

    std::fs::write(path, new_content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_update_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");
        std::fs::write(&path, "").unwrap();

        update_changelog(&path, "## 1.0.0\n\n- Initial release\n\n").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "# Changelog\n\n## 1.0.0\n\n- Initial release\n\n");
    }

    #[test]
    fn test_update_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");

        update_changelog(&path, "## 1.0.0\n\n- First\n\n").unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "# Changelog\n\n## 1.0.0\n\n- First\n\n");
    }

    #[test]
    fn test_update_with_existing_header() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");
        std::fs::write(&path, "# Changelog\n\nold content\n").unwrap();

        update_changelog(&path, "## 2.0.0\n\n- New stuff\n\n").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "# Changelog\n\n## 2.0.0\n\n- New stuff\n\nold content\n"
        );
    }

    #[test]
    fn test_update_without_header() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");
        std::fs::write(&path, "some existing content\n").unwrap();

        update_changelog(&path, "## 1.0.0\n\n- Entry\n\n").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "# Changelog\n\n## 1.0.0\n\n- Entry\n\nsome existing content\n"
        );
    }

    #[test]
    fn test_multiple_sequential_updates() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("CHANGELOG.md");

        update_changelog(&path, "## 1.0.0\n\n- First release\n\n").unwrap();
        update_changelog(&path, "## 2.0.0\n\n- Second release\n\n").unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "# Changelog\n\n## 2.0.0\n\n- Second release\n\n## 1.0.0\n\n- First release\n\n"
        );
    }

    use crate::BumpType;
    use crate::changelog_entry::{Changelog, Release};
    use crate::plan::PackageRelease;
    use semver::Version;

    #[test]
    fn test_generate_entry_single_patch() {
        let dir = TempDir::new().unwrap();
        let release = PackageRelease {
            name: "foo".to_string(),
            bump: BumpType::Patch,
            old_version: Version::new(1, 0, 0),
            new_version: Version::new(1, 0, 1),
            changelog_ids: vec!["change-1".to_string()],
        };
        let changelogs = vec![Changelog {
            id: "change-1".to_string(),
            summary: "fix a bug".to_string(),
            releases: vec![Release {
                package: "foo".to_string(),
                bump: BumpType::Patch,
            }],
            commit: None,
        }];

        let output = generate_entry(&release, &changelogs, dir.path());

        assert!(output.contains("## 1.0.1 ("));
        assert!(output.contains("### Patch Changes"));
        assert!(output.contains("fix a bug"));
    }

    #[test]
    fn test_generate_entry_multiple_bump_types() {
        let dir = TempDir::new().unwrap();
        let release = PackageRelease {
            name: "foo".to_string(),
            bump: BumpType::Major,
            old_version: Version::new(1, 0, 0),
            new_version: Version::new(2, 0, 0),
            changelog_ids: vec![
                "c-major".to_string(),
                "c-minor".to_string(),
                "c-patch".to_string(),
            ],
        };
        let changelogs = vec![
            Changelog {
                id: "c-major".to_string(),
                summary: "breaking api change".to_string(),
                releases: vec![Release {
                    package: "foo".to_string(),
                    bump: BumpType::Major,
                }],
                commit: None,
            },
            Changelog {
                id: "c-minor".to_string(),
                summary: "new feature".to_string(),
                releases: vec![Release {
                    package: "foo".to_string(),
                    bump: BumpType::Minor,
                }],
                commit: None,
            },
            Changelog {
                id: "c-patch".to_string(),
                summary: "small fix".to_string(),
                releases: vec![Release {
                    package: "foo".to_string(),
                    bump: BumpType::Patch,
                }],
                commit: None,
            },
        ];

        let output = generate_entry(&release, &changelogs, dir.path());

        assert!(output.contains("### Major Changes"));
        assert!(output.contains("### Minor Changes"));
        assert!(output.contains("### Patch Changes"));
        assert!(output.contains("breaking api change"));
        assert!(output.contains("new feature"));
        assert!(output.contains("small fix"));

        let major_pos = output.find("### Major Changes").unwrap();
        let minor_pos = output.find("### Minor Changes").unwrap();
        let patch_pos = output.find("### Patch Changes").unwrap();
        assert!(major_pos < minor_pos);
        assert!(minor_pos < patch_pos);
    }

    #[test]
    fn test_generate_entry_only_major() {
        let dir = TempDir::new().unwrap();
        let release = PackageRelease {
            name: "foo".to_string(),
            bump: BumpType::Major,
            old_version: Version::new(1, 0, 0),
            new_version: Version::new(2, 0, 0),
            changelog_ids: vec!["c-1".to_string()],
        };
        let changelogs = vec![Changelog {
            id: "c-1".to_string(),
            summary: "removed old api".to_string(),
            releases: vec![Release {
                package: "foo".to_string(),
                bump: BumpType::Major,
            }],
            commit: None,
        }];

        let output = generate_entry(&release, &changelogs, dir.path());

        assert!(output.contains("### Major Changes"));
        assert!(!output.contains("### Minor Changes"));
        assert!(!output.contains("### Patch Changes"));
    }

    #[test]
    fn test_generate_entry_multiline_summary() {
        let dir = TempDir::new().unwrap();
        let release = PackageRelease {
            name: "foo".to_string(),
            bump: BumpType::Minor,
            old_version: Version::new(1, 0, 0),
            new_version: Version::new(1, 1, 0),
            changelog_ids: vec!["c-1".to_string()],
        };
        let changelogs = vec![Changelog {
            id: "c-1".to_string(),
            summary: "added new feature\nwith detailed explanation\nand examples".to_string(),
            releases: vec![Release {
                package: "foo".to_string(),
                bump: BumpType::Minor,
            }],
            commit: None,
        }];

        let output = generate_entry(&release, &changelogs, dir.path());

        assert!(output.contains("added new feature"));
        assert!(output.contains("with detailed explanation"));
        assert!(output.contains("and examples"));
    }

    #[test]
    fn test_generate_entry_no_matching_changelogs() {
        let dir = TempDir::new().unwrap();
        let release = PackageRelease {
            name: "foo".to_string(),
            bump: BumpType::Patch,
            old_version: Version::new(1, 0, 0),
            new_version: Version::new(1, 0, 1),
            changelog_ids: vec!["nonexistent".to_string()],
        };
        let changelogs = vec![Changelog {
            id: "other-change".to_string(),
            summary: "unrelated change".to_string(),
            releases: vec![Release {
                package: "foo".to_string(),
                bump: BumpType::Patch,
            }],
            commit: None,
        }];

        let output = generate_entry(&release, &changelogs, dir.path());

        assert!(output.contains("## 1.0.1 ("));
        assert!(!output.contains("### Major Changes"));
        assert!(!output.contains("### Minor Changes"));
        assert!(!output.contains("### Patch Changes"));
    }
}

pub fn write_changelogs(
    workspace: &Workspace,
    releases: &[PackageRelease],
    changelogs: &[Changelog],
    format: ChangelogFormat,
) -> Result<()> {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    write_changelogs_with_date(workspace, releases, changelogs, format, &date)
}

pub fn write_changelogs_with_date(
    workspace: &Workspace,
    releases: &[PackageRelease],
    changelogs: &[Changelog],
    format: ChangelogFormat,
    date: &str,
) -> Result<()> {
    let changelog_dir = &workspace.changelog_dir;

    match format {
        ChangelogFormat::PerCrate => {
            for release in releases {
                if let Some(package) = workspace.get_package(&release.name) {
                    let mut entry = format!("## `{}@{}`\n\n", release.name, release.new_version);
                    let generated =
                        generate_entry_with_date(release, changelogs, changelog_dir, date);
                    let entry_body = generated.lines().skip(2).collect::<Vec<_>>().join("\n");
                    entry.push_str(&entry_body);
                    entry.push('\n');

                    let changelog_path = package.path.join("CHANGELOG.md");
                    update_changelog(&changelog_path, &entry)?;
                }
            }
        }
        ChangelogFormat::Root => {
            // Group releases by version so fixed-group packages sharing the same
            // version get a single heading instead of duplicate `## version` blocks.
            let mut by_version: BTreeMap<String, Vec<&PackageRelease>> = BTreeMap::new();
            for release in releases {
                by_version
                    .entry(release.new_version.to_string())
                    .or_default()
                    .push(release);
            }

            let mut combined_entry = String::new();

            for (version, group) in &by_version {
                if group.len() == 1 {
                    // Single release at this version — use existing per-package generation.
                    let entry = generate_entry_with_date(group[0], changelogs, changelog_dir, date);
                    combined_entry.push_str(&entry);
                } else {
                    // Multiple releases share this version — merge into one heading
                    // and deduplicate changelog entries that appear in multiple packages.
                    combined_entry.push_str(&format!("## {} ({})\n\n", version, date));

                    let github_url = get_github_url();

                    let mut major_changes = Vec::new();
                    let mut minor_changes = Vec::new();
                    let mut patch_changes = Vec::new();
                    let mut seen_changelog_ids: HashSet<&str> = HashSet::new();

                    for release in group {
                        for changelog in changelogs {
                            if !release.changelog_ids.contains(&changelog.id) {
                                continue;
                            }
                            if !seen_changelog_ids.insert(&changelog.id) {
                                continue;
                            }

                            // Find the highest bump level for this changelog across
                            // all packages in the group.
                            let bump = group
                                .iter()
                                .filter(|r| r.changelog_ids.contains(&changelog.id))
                                .flat_map(|r| {
                                    changelog
                                        .releases
                                        .iter()
                                        .filter(|rel| rel.package == r.name)
                                        .map(|rel| rel.bump)
                                })
                                .max()
                                .unwrap_or(BumpType::Patch);

                            let summary = changelog.summary.trim().to_string();

                            let (link_info, authors) = github_url
                                .as_ref()
                                .and_then(|base| {
                                    let info = changelog_entry::get_commit_info(
                                        changelog_dir,
                                        &changelog.id,
                                    )?;
                                    let link_info = if let Some(pr) = info.pr_number {
                                        Some((format!("{}/pull/{}", base, pr), format!("#{}", pr)))
                                    } else {
                                        let short_sha =
                                            &info.commit_sha[..7.min(info.commit_sha.len())];
                                        Some((
                                            format!("{}/commit/{}", base, short_sha),
                                            short_sha.to_string(),
                                        ))
                                    };
                                    Some((link_info, info.authors))
                                })
                                .unwrap_or((None, Vec::new()));

                            let change = ChangeWithMeta {
                                summary,
                                link: link_info,
                                authors,
                            };
                            match bump {
                                BumpType::Major => major_changes.push(change),
                                BumpType::Minor => minor_changes.push(change),
                                BumpType::Patch => patch_changes.push(change),
                            }
                        }
                    }

                    if !major_changes.is_empty() {
                        combined_entry.push_str("### Major Changes\n\n");
                        for change in major_changes {
                            write_change_lines(&mut combined_entry, &change);
                        }
                        combined_entry.push('\n');
                    }
                    if !minor_changes.is_empty() {
                        combined_entry.push_str("### Minor Changes\n\n");
                        for change in minor_changes {
                            write_change_lines(&mut combined_entry, &change);
                        }
                        combined_entry.push('\n');
                    }
                    if !patch_changes.is_empty() {
                        combined_entry.push_str("### Patch Changes\n\n");
                        for change in patch_changes {
                            write_change_lines(&mut combined_entry, &change);
                        }
                        combined_entry.push('\n');
                    }
                }
            }

            let changelog_path = workspace.root.join("CHANGELOG.md");
            update_changelog(&changelog_path, &combined_entry)?;
        }
    }

    Ok(())
}
