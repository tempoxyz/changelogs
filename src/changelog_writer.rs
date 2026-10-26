use crate::BumpType;
use crate::changelog_entry::{self, Changelog};
use crate::config::ChangelogFormat;
use crate::error::Result;
use crate::plan::PackageRelease;
use crate::workspace::Workspace;
use chrono::Utc;
use semver::Version;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn get_github_url(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(dir)
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

/// The link and authors shown after a change, from the commit that added its
/// changelog entry.
#[derive(Debug, Default)]
struct ChangeMeta {
    link: Option<(String, String)>, // (url, display_text)
    authors: Vec<String>,
}

/// Looks up the commit, PR and authors of each changelog entry once: one
/// `git remote` call per run and one commit lookup per entry, however many
/// packages the entry releases.
fn resolve_metadata<'a>(
    changelog_dir: &Path,
    ids: impl IntoIterator<Item = &'a str>,
) -> HashMap<&'a str, ChangeMeta> {
    let mut metadata = HashMap::new();
    let mut ids = ids.into_iter().peekable();
    if ids.peek().is_none() {
        return metadata;
    }
    let Some(base) = get_github_url(changelog_dir) else {
        return metadata;
    };

    for id in ids {
        metadata.entry(id).or_insert_with(|| {
            let Some(info) = changelog_entry::get_commit_info(changelog_dir, id) else {
                return ChangeMeta::default();
            };
            let link = if let Some(pr) = info.pr_number {
                (format!("{}/pull/{}", base, pr), format!("#{}", pr))
            } else {
                let short_sha = &info.commit_sha[..7.min(info.commit_sha.len())];
                (
                    format!("{}/commit/{}", base, short_sha),
                    short_sha.to_string(),
                )
            };
            ChangeMeta {
                link: Some(link),
                authors: info.authors,
            }
        });
    }
    metadata
}

/// The changelog entries a section lists, each with the bump it is listed under.
type Changes<'a> = Vec<(BumpType, &'a Changelog)>;

/// The entries of one package's release, under the bump each gives it.
fn package_changes<'a>(release: &PackageRelease, changelogs: &'a [Changelog]) -> Changes<'a> {
    changelogs
        .iter()
        .filter(|changelog| release.changelog_ids.contains(&changelog.id))
        .flat_map(|changelog| {
            changelog
                .releases
                .iter()
                .filter(|rel| rel.package == release.name)
                .map(move |rel| (rel.bump, changelog))
        })
        .collect()
}

/// The entries of releases sharing one version, each listed once under the
/// highest bump it gives any package in the group.
fn merged_changes<'a>(group: &[&PackageRelease], changelogs: &'a [Changelog]) -> Changes<'a> {
    let mut seen_changelog_ids: HashSet<&str> = HashSet::new();
    let mut changes = Vec::new();

    for release in group {
        for changelog in changelogs {
            if !release.changelog_ids.contains(&changelog.id)
                || !seen_changelog_ids.insert(&changelog.id)
            {
                continue;
            }

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
            changes.push((bump, changelog));
        }
    }

    changes
}

/// Appends the Major, Minor and Patch Changes subsections listing `changes`.
fn render_changes(entry: &mut String, changes: &Changes, metadata: &HashMap<&str, ChangeMeta>) {
    let no_meta = ChangeMeta::default();
    for (bump, title) in [
        (BumpType::Major, "### Major Changes\n\n"),
        (BumpType::Minor, "### Minor Changes\n\n"),
        (BumpType::Patch, "### Patch Changes\n\n"),
    ] {
        let mut listed = changes.iter().filter(|(b, _)| *b == bump).peekable();
        if listed.peek().is_none() {
            continue;
        }
        entry.push_str(title);
        for (_, changelog) in listed {
            let meta = metadata.get(changelog.id.as_str()).unwrap_or(&no_meta);
            write_change_lines(entry, changelog.summary.trim(), meta);
        }
        entry.push('\n');
    }
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
    let changes = package_changes(release, changelogs);
    let metadata = resolve_metadata(changelog_dir, changes.iter().map(|(_, c)| c.id.as_str()));
    let mut entry = format!("## {} ({})\n\n", release.new_version, date);
    render_changes(&mut entry, &changes, &metadata);
    entry
}

fn write_change_lines(entry: &mut String, summary: &str, meta: &ChangeMeta) {
    let mut suffix_parts = Vec::new();

    if !meta.authors.is_empty() {
        let authors_str = meta.authors.join(", ");
        suffix_parts.push(format!("by {}", authors_str));
    }

    if let Some((ref url, ref display)) = meta.link {
        suffix_parts.push(format!("[{}]({})", display, url));
    }

    let suffix = if suffix_parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", suffix_parts.join(", "))
    };

    let lines: Vec<&str> = summary.lines().collect();
    let mut in_fence = false;
    for (i, line) in lines.iter().enumerate() {
        let is_last = i == lines.len() - 1;
        let line_suffix = if is_last { &suffix } else { "" };

        if line.starts_with("```") || line.starts_with("~~~") {
            // Keep fenced blocks inside the current changelog list item. In
            // particular, diff lines beginning with `-` must not become new
            // top-level list items, and metadata must not trail the closing
            // fence or it will stop being recognized as a fence.
            if i == 0 {
                entry.push_str("-\n");
            }
            entry.push_str(&format!("  {}\n", line));
            in_fence = !in_fence;
            if is_last && !suffix.is_empty() {
                entry.push_str(&format!("  {}\n", suffix.trim_start()));
            }
        } else if in_fence {
            entry.push_str(&format!("  {}\n", line));
            if is_last && !suffix.is_empty() {
                entry.push_str(&format!("  {}\n", suffix.trim_start()));
            }
        } else if line.starts_with('-') || line.starts_with('*') {
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
    let sections = render_sections_with_date(workspace, releases, changelogs, format, date);
    write_sections(&sections, format)
}

/// Renders the changelog sections `write_changelogs` would add, dated today.
pub fn render_sections(
    workspace: &Workspace,
    releases: &[PackageRelease],
    changelogs: &[Changelog],
    format: ChangelogFormat,
) -> Vec<Section> {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    render_sections_with_date(workspace, releases, changelogs, format, &date)
}

/// A changelog section that `version` prepends to a CHANGELOG.md file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The CHANGELOG.md file the section goes into.
    pub path: PathBuf,
    /// The packages whose release the section describes.
    pub packages: Vec<String>,
    /// The section's markdown, starting with its `## ` heading.
    pub text: String,
}

/// Renders the changelog sections for `releases` without writing them.
pub fn render_sections_with_date(
    workspace: &Workspace,
    releases: &[PackageRelease],
    changelogs: &[Changelog],
    format: ChangelogFormat,
    date: &str,
) -> Vec<Section> {
    // Each section with its heading and the entries it lists.
    let mut planned: Vec<(Section, Changes)> = Vec::new();

    match format {
        ChangelogFormat::PerCrate => {
            for release in releases {
                if let Some(package) = workspace.get_package(&release.name) {
                    let section = Section {
                        path: package.path.join("CHANGELOG.md"),
                        packages: vec![release.name.clone()],
                        text: format!("## `{}@{}`\n\n", release.name, release.new_version),
                    };
                    planned.push((section, package_changes(release, changelogs)));
                }
            }
        }
        ChangelogFormat::Root => {
            // Group releases by version so fixed-group packages sharing the same
            // version get a single heading instead of duplicate `## version` blocks.
            // Releases without changelog entries (such as dependent bumps) get
            // no heading of their own.
            let mut by_version: BTreeMap<&Version, Vec<&PackageRelease>> = BTreeMap::new();
            for release in releases {
                if release.changelog_ids.is_empty() {
                    continue;
                }
                by_version
                    .entry(&release.new_version)
                    .or_default()
                    .push(release);
            }

            // Newest version first, matching how entries are prepended.
            for (version, group) in by_version.iter().rev() {
                // Multiple releases sharing a version merge into one heading
                // and list each changelog entry once.
                let changes = if group.len() == 1 {
                    package_changes(group[0], changelogs)
                } else {
                    merged_changes(group, changelogs)
                };
                let section = Section {
                    path: workspace.root.join("CHANGELOG.md"),
                    packages: group.iter().map(|release| release.name.clone()).collect(),
                    text: format!("## {} ({})\n\n", version, date),
                };
                planned.push((section, changes));
            }
        }
    }

    let metadata = resolve_metadata(
        &workspace.changelog_dir(),
        planned
            .iter()
            .flat_map(|(_, changes)| changes.iter().map(|(_, c)| c.id.as_str())),
    );

    planned
        .into_iter()
        .map(|(mut section, changes)| {
            let heading_len = section.text.len();
            render_changes(&mut section.text, &changes, &metadata);
            if format == ChangelogFormat::PerCrate && section.text.len() == heading_len {
                // A per-crate section without entries keeps its blank line.
                section.text.push('\n');
            }
            section
        })
        .collect()
}

/// Prepends rendered sections to their CHANGELOG.md files.
pub fn write_sections(sections: &[Section], format: ChangelogFormat) -> Result<()> {
    match format {
        ChangelogFormat::PerCrate => {
            for section in sections {
                update_changelog(&section.path, &section.text)?;
            }
        }
        ChangelogFormat::Root => {
            let combined_entry: String = sections.iter().map(|s| s.text.as_str()).collect();
            if let Some(section) = sections.first() {
                update_changelog(&section.path, &combined_entry)?;
            }
        }
    }

    Ok(())
}

/// Returns the body of the CHANGELOG.md section for `version` (without its
/// heading), matching both per-crate (`` ## `name@version` ``) and root
/// (`## version (date)`) headings.
pub fn find_section(content: &str, name: &str, version: &Version) -> Option<String> {
    let per_crate = format!("## `{}@{}`", name, version);
    let root = format!("## {}", version);
    let mut lines = content.lines();
    lines.find(|line| {
        line.trim_end() == per_crate
            || line.strip_prefix(&root).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(' ') || rest.starts_with('(')
            })
    })?;
    let body: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_string())
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
    fn test_write_change_lines_keeps_diff_fence_in_list_item() {
        let summary = "Changed the configuration.\n```diff\n-old = true\n+new = true\n```";
        let meta = ChangeMeta {
            link: Some((
                "https://github.com/example/repo/pull/123".to_string(),
                "#123".to_string(),
            )),
            authors: vec!["@octocat".to_string()],
        };
        let mut output = String::new();

        write_change_lines(&mut output, summary, &meta);

        assert_eq!(
            output,
            "- Changed the configuration.\n  ```diff\n  -old = true\n  +new = true\n  ```\n  (by @octocat, [#123](https://github.com/example/repo/pull/123))\n"
        );
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

    #[test]
    fn test_root_format_orders_versions_newest_first_and_skips_empty() {
        let dir = TempDir::new().unwrap();
        let release = |name: &str, version: &str, ids: &[&str]| PackageRelease {
            name: name.to_string(),
            bump: BumpType::Major,
            old_version: Version::new(0, 1, 0),
            new_version: Version::parse(version).unwrap(),
            changelog_ids: ids.iter().map(|id| id.to_string()).collect(),
        };
        let changelog = |id: &str, package: &str| Changelog {
            id: id.to_string(),
            summary: format!("change in {package}"),
            releases: vec![Release {
                package: package.to_string(),
                bump: BumpType::Major,
            }],
            commit: None,
        };
        let releases = vec![
            release("a", "1.0.0", &["c-a"]),
            release("b", "10.0.0", &["c-b"]),
            release("c", "2.0.0", &["c-c"]),
            release("dependent", "3.0.0", &[]),
        ];
        let changelogs = vec![
            changelog("c-a", "a"),
            changelog("c-b", "b"),
            changelog("c-c", "c"),
        ];
        let workspace = Workspace {
            root: dir.path().to_path_buf(),
            packages: Vec::new(),
            ecosystem: crate::ecosystems::Ecosystem::Rust,
            config: Default::default(),
        };

        write_changelogs_with_date(
            &workspace,
            &releases,
            &changelogs,
            ChangelogFormat::Root,
            "2025-01-15",
        )
        .unwrap();

        let content = std::fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
        let headings: Vec<&str> = content.lines().filter(|l| l.starts_with("## ")).collect();
        assert_eq!(
            headings,
            vec![
                "## 10.0.0 (2025-01-15)",
                "## 2.0.0 (2025-01-15)",
                "## 1.0.0 (2025-01-15)",
            ]
        );
    }

    #[test]
    fn test_root_format_without_entries_leaves_changelog_untouched() {
        let dir = TempDir::new().unwrap();
        let workspace = Workspace {
            root: dir.path().to_path_buf(),
            packages: Vec::new(),
            ecosystem: crate::ecosystems::Ecosystem::Rust,
            config: Default::default(),
        };
        let releases = vec![PackageRelease {
            name: "dependent".to_string(),
            bump: BumpType::Patch,
            old_version: Version::new(1, 0, 0),
            new_version: Version::new(1, 0, 1),
            changelog_ids: Vec::new(),
        }];

        write_changelogs_with_date(
            &workspace,
            &releases,
            &[],
            ChangelogFormat::Root,
            "2025-01-15",
        )
        .unwrap();

        assert!(!dir.path().join("CHANGELOG.md").exists());
    }

    #[test]
    fn find_section_matches_exact_release() {
        let content = "# Changelog\n\n## 1.1.0-rc1 (2025-01-20)\n\n- rc\n\n## 1.1.0 (2025-01-15)\n\n- stable\n\n## `lib@0.2.0`\n\n### Patch Changes\n\n- lib fix\n\n## 1.0.0\n\n- old\n";
        let section = |name, version: &str| find_section(content, name, &version.parse().unwrap());

        assert_eq!(section("app", "1.1.0").as_deref(), Some("- stable"));
        assert_eq!(section("app", "1.1.0-rc1").as_deref(), Some("- rc"));
        assert_eq!(
            section("lib", "0.2.0").as_deref(),
            Some("### Patch Changes\n\n- lib fix")
        );
        assert_eq!(section("app", "1.0.0").as_deref(), Some("- old"));
        assert_eq!(section("app", "2.0.0"), None);
    }
}
