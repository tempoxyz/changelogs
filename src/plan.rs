use crate::BumpType;
use crate::changelog_entry::Changelog;
use crate::config::{ChangelogFormat, Config, DependentBump};
use crate::graph::DependencyGraph;
use crate::workspace::Workspace;
use semver::{Prerelease, Version};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ReleasePlan {
    pub changelogs: Vec<Changelog>,
    pub releases: Vec<PackageRelease>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PackageRelease {
    pub name: String,
    pub bump: BumpType,
    pub old_version: Version,
    pub new_version: Version,
    pub changelog_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrereleasePrefix(String);

impl PrereleasePrefix {
    pub fn new(prefix: impl Into<String>) -> Result<Self, String> {
        let prefix = prefix.into();
        if prefix.is_empty() {
            return Err("prerelease identifier cannot be empty".to_string());
        }

        let candidate = format!("{}1", prefix);
        Prerelease::new(&candidate)
            .map_err(|e| format!("invalid prerelease identifier '{}': {}", prefix, e))?;

        Ok(Self(prefix))
    }

    fn next(&self, current: Option<&Prerelease>) -> Prerelease {
        let number = current
            .and_then(|pre| pre.as_str().strip_prefix(&self.0))
            .and_then(|suffix| suffix.parse::<u64>().ok())
            .and_then(|n| n.checked_add(1))
            .unwrap_or(1);

        Prerelease::new(&format!("{}{number}", self.0)).expect("validated prerelease identifier")
    }
}

pub fn bump_version(version: &Version, bump: BumpType) -> Version {
    match bump {
        BumpType::Major => Version::new(version.major + 1, 0, 0),
        BumpType::Minor => Version::new(version.major, version.minor + 1, 0),
        BumpType::Patch => Version::new(version.major, version.minor, version.patch + 1),
    }
}

/// The bump a prerelease cycle is heading towards: `2.0.0-rc1` is a major
/// cycle, `1.3.0-rc1` a minor one and `1.3.1-rc1` a patch one.
fn prerelease_cycle_bump(version: &Version) -> BumpType {
    if version.minor == 0 && version.patch == 0 {
        BumpType::Major
    } else if version.patch == 0 {
        BumpType::Minor
    } else {
        BumpType::Patch
    }
}

pub fn bump_version_with_prerelease(
    version: &Version,
    bump: BumpType,
    prerelease: Option<&PrereleasePrefix>,
) -> Version {
    let base = Version::new(version.major, version.minor, version.patch);
    // A bump larger than the current prerelease cycle implies starts a new
    // cycle (`1.3.0-rc1` + major => `2.0.0`), otherwise the cycle continues.
    let starts_new_cycle = version.pre.is_empty() || bump > prerelease_cycle_bump(version);

    match prerelease {
        Some(prefix) => {
            if starts_new_cycle {
                let mut next = bump_version(&base, bump);
                next.pre = prefix.next(None);
                next
            } else {
                let mut next = base;
                next.pre = prefix.next(Some(&version.pre));
                next
            }
        }
        None if starts_new_cycle => bump_version(&base, bump),
        None => base,
    }
}

pub fn assemble(workspace: &Workspace, changelogs: Vec<Changelog>, config: &Config) -> ReleasePlan {
    assemble_with_prerelease(workspace, changelogs, config, None)
}

pub fn assemble_stable_promotions(workspace: &Workspace, config: &Config) -> ReleasePlan {
    let mut releases = workspace
        .packages
        .iter()
        .filter(|pkg| !config.ignore.contains(&pkg.name))
        .filter(|pkg| !pkg.version.pre.is_empty())
        .map(|pkg| PackageRelease {
            name: pkg.name.clone(),
            bump: BumpType::Patch,
            old_version: pkg.version.clone(),
            new_version: Version::new(pkg.version.major, pkg.version.minor, pkg.version.patch),
            changelog_ids: Vec::new(),
        })
        .collect::<Vec<_>>();

    releases.sort_by(|a, b| a.name.cmp(&b.name));

    ReleasePlan {
        changelogs: Vec::new(),
        releases,
        warnings: Vec::new(),
    }
}

/// Packages referenced by changelog entries that are neither in the workspace
/// nor listed in the config's `ignore`, as `(changelog id, package)` pairs.
pub fn unknown_package_refs(
    workspace: &Workspace,
    changelogs: &[Changelog],
    config: &Config,
) -> Vec<(String, String)> {
    changelogs
        .iter()
        .flat_map(|changelog| {
            changelog
                .releases
                .iter()
                .filter(|release| {
                    !config.ignore.contains(&release.package)
                        && workspace.get_package(&release.package).is_none()
                })
                .map(|release| (changelog.id.clone(), release.package.clone()))
        })
        .collect()
}

pub fn assemble_with_prerelease(
    workspace: &Workspace,
    changelogs: Vec<Changelog>,
    config: &Config,
    prerelease: Option<&PrereleasePrefix>,
) -> ReleasePlan {
    let graph = DependencyGraph::from_workspace(workspace);

    let mut bump_map: HashMap<String, BumpType> = HashMap::new();
    let mut changelog_map: HashMap<String, Vec<String>> = HashMap::new();

    for changelog in &changelogs {
        for release in &changelog.releases {
            if config.ignore.contains(&release.package) {
                continue;
            }

            let current = bump_map.get(&release.package).copied();
            let new_bump = match current {
                Some(existing) => existing.max(release.bump),
                None => release.bump,
            };
            bump_map.insert(release.package.clone(), new_bump);

            changelog_map
                .entry(release.package.clone())
                .or_default()
                .push(changelog.id.clone());
        }
    }

    // Build effective fixed groups: explicit ones plus, when using root format,
    // an implicit group of all non-ignored workspace packages.
    let mut fixed_groups: Vec<Vec<String>> =
        config.fixed.iter().map(|g| g.members.clone()).collect();

    if config.changelog.format == ChangelogFormat::Root {
        let all_members: Vec<String> = workspace
            .package_names()
            .into_iter()
            .filter(|n| !config.ignore.contains(&n.to_string()))
            .map(|n| n.to_string())
            .collect();
        if all_members.len() > 1 {
            fixed_groups.push(all_members);
        }
    }

    let dependent_bump_type = match config.dependent_bump {
        DependentBump::Patch => Some(BumpType::Patch),
        DependentBump::Minor => Some(BumpType::Minor),
        DependentBump::None => None,
    };

    // Dependent bumps can pull a package into a fixed or linked group, and a
    // group can raise a package whose dependents then need a bump, so apply
    // all rules until the bump map stops changing. Bumps only ever grow, so
    // this terminates.
    loop {
        let before = bump_map.clone();

        if let Some(dependent_bump_type) = dependent_bump_type {
            let changed_packages: Vec<String> = bump_map.keys().cloned().collect();

            for pkg in changed_packages {
                for dependent in graph.all_dependents(&pkg) {
                    if config.ignore.contains(&dependent) {
                        continue;
                    }

                    let current = bump_map.get(&dependent).copied();
                    match current {
                        Some(existing) if existing >= dependent_bump_type => {}
                        _ => {
                            bump_map.insert(dependent, dependent_bump_type);
                        }
                    }
                }
            }
        }

        for members in &fixed_groups {
            let max_bump = members
                .iter()
                .filter_map(|m| bump_map.get(m))
                .max()
                .copied();

            if let Some(bump) = max_bump {
                for member in members {
                    if !config.ignore.contains(member) {
                        bump_map.insert(member.clone(), bump);
                    }
                }
            }
        }

        for group in &config.linked {
            let releasing: Vec<_> = group
                .members
                .iter()
                .filter(|m| bump_map.contains_key(*m))
                .collect();

            if releasing.len() > 1 {
                let max_bump = releasing
                    .iter()
                    .filter_map(|m| bump_map.get(*m))
                    .max()
                    .copied();

                if let Some(bump) = max_bump {
                    for member in releasing {
                        bump_map.insert(member.clone(), bump);
                    }
                }
            }
        }

        if bump_map == before {
            break;
        }
    }

    let mut warnings: Vec<String> = Vec::new();
    let mut releases: Vec<PackageRelease> = Vec::new();

    for (name, bump) in bump_map {
        if let Some(package) = workspace.get_package(&name) {
            let new_version = bump_version_with_prerelease(&package.version, bump, prerelease);
            releases.push(PackageRelease {
                name: name.clone(),
                bump,
                old_version: package.version.clone(),
                new_version,
                changelog_ids: changelog_map.remove(&name).unwrap_or_default(),
            });
        } else {
            warnings.push(format!("changelog references unknown package '{}'", name));
        }
    }

    // Fixed groups always release on one version: the highest version any
    // member ends up on, even when members start on different versions.
    for members in &fixed_groups {
        let Some(group_version) = releases
            .iter()
            .filter(|r| members.contains(&r.name))
            .map(|r| r.new_version.clone())
            .max()
        else {
            continue;
        };
        for release in releases.iter_mut().filter(|r| members.contains(&r.name)) {
            release.new_version = group_version.clone();
        }
    }

    releases.sort_by(|a, b| a.name.cmp(&b.name));
    warnings.sort();

    ReleasePlan {
        changelogs,
        releases,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Release;
    use semver::Version;

    fn mock_package(name: &str, version: &str, deps: Vec<&str>) -> crate::ecosystems::Package {
        crate::ecosystems::Package {
            name: name.to_string(),
            version: Version::parse(version).unwrap(),
            path: std::path::PathBuf::from(format!("crates/{}", name)),
            manifest_path: std::path::PathBuf::from(format!("crates/{}/Cargo.toml", name)),
            dependencies: deps.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn test_highest_bump_wins() {
        let ws = mock_workspace(vec![mock_package("foo", "1.0.0", vec![])]);
        let changelogs = vec![
            make_changelog(
                "a",
                vec![Release {
                    package: "foo".to_string(),
                    bump: BumpType::Patch,
                }],
            ),
            make_changelog(
                "b",
                vec![Release {
                    package: "foo".to_string(),
                    bump: BumpType::Minor,
                }],
            ),
        ];

        let plan = assemble(&ws, changelogs, &Config::default());

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].bump, BumpType::Minor);
        assert_eq!(plan.releases[0].new_version, Version::new(1, 1, 0));
    }

    fn mock_workspace(packages: Vec<crate::ecosystems::Package>) -> Workspace {
        Workspace {
            root: std::path::PathBuf::from("/tmp/test"),
            changelog_dir: std::path::PathBuf::from("/tmp/test/.changelog"),
            packages,
            ecosystem: crate::ecosystems::Ecosystem::Rust,
        }
    }

    fn make_changelog(id: &str, releases: Vec<Release>) -> Changelog {
        Changelog {
            id: id.to_string(),
            summary: format!("changelog {}", id),
            releases,
            commit: None,
        }
    }

    #[test]
    fn test_assemble_simple_bump() {
        let ws = mock_workspace(vec![mock_package("foo", "1.0.0", vec![])]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "foo".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config::default();

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].name, "foo");
        assert_eq!(plan.releases[0].bump, BumpType::Minor);
        assert_eq!(plan.releases[0].old_version, Version::new(1, 0, 0));
        assert_eq!(plan.releases[0].new_version, Version::new(1, 1, 0));
    }

    #[test]
    fn test_prerelease_bump_starts_at_one() {
        let rc = PrereleasePrefix::new("rc").unwrap();
        let version = bump_version_with_prerelease(
            &Version::parse("1.5.0").unwrap(),
            BumpType::Minor,
            Some(&rc),
        );

        assert_eq!(version, Version::parse("1.6.0-rc1").unwrap());
    }

    #[test]
    fn test_prerelease_bump_increments_existing_suffix() {
        let rc = PrereleasePrefix::new("rc").unwrap();
        let version = bump_version_with_prerelease(
            &Version::parse("1.6.0-rc1").unwrap(),
            BumpType::Patch,
            Some(&rc),
        );

        assert_eq!(version, Version::parse("1.6.0-rc2").unwrap());
    }

    #[test]
    fn test_stable_bump_promotes_prerelease() {
        let version = bump_version_with_prerelease(
            &Version::parse("1.6.0-rc2").unwrap(),
            BumpType::Patch,
            None,
        );

        assert_eq!(version, Version::parse("1.6.0").unwrap());
    }

    #[test]
    fn test_assemble_stable_promotions() {
        let ws = mock_workspace(vec![
            mock_package("foo", "1.6.0-rc2", vec![]),
            mock_package("bar", "1.0.0", vec![]),
        ]);
        let config = Config::default();

        let plan = assemble_stable_promotions(&ws, &config);

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].name, "foo");
        assert_eq!(
            plan.releases[0].old_version,
            Version::parse("1.6.0-rc2").unwrap()
        );
        assert_eq!(
            plan.releases[0].new_version,
            Version::parse("1.6.0").unwrap()
        );
    }

    #[test]
    fn test_assemble_stable_promotions_respects_ignore() {
        let ws = mock_workspace(vec![mock_package("foo", "1.6.0-rc2", vec![])]);
        let config = Config {
            ignore: vec!["foo".to_string()],
            ..Config::default()
        };

        let plan = assemble_stable_promotions(&ws, &config);

        assert!(plan.releases.is_empty());
    }

    #[test]
    fn test_assemble_with_prerelease() {
        let ws = mock_workspace(vec![mock_package("foo", "1.0.0", vec![])]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "foo".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config::default();
        let rc = PrereleasePrefix::new("rc").unwrap();

        let plan = assemble_with_prerelease(&ws, changelogs, &config, Some(&rc));

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(
            plan.releases[0].new_version,
            Version::parse("1.1.0-rc1").unwrap()
        );
    }

    #[test]
    fn test_assemble_dependent_bump_patch() {
        let ws = mock_workspace(vec![
            mock_package("a", "1.0.0", vec![]),
            mock_package("b", "2.0.0", vec!["a"]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "a".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config::default();

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 2);
        let a = plan.releases.iter().find(|r| r.name == "a").unwrap();
        let b = plan.releases.iter().find(|r| r.name == "b").unwrap();
        assert_eq!(a.bump, BumpType::Minor);
        assert_eq!(a.new_version, Version::new(1, 1, 0));
        assert_eq!(b.bump, BumpType::Patch);
        assert_eq!(b.new_version, Version::new(2, 0, 1));
    }

    #[test]
    fn test_assemble_dependent_bump_minor() {
        let ws = mock_workspace(vec![
            mock_package("a", "1.0.0", vec![]),
            mock_package("b", "2.0.0", vec!["a"]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "a".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config {
            dependent_bump: DependentBump::Minor,
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        let b = plan.releases.iter().find(|r| r.name == "b").unwrap();
        assert_eq!(b.bump, BumpType::Minor);
        assert_eq!(b.new_version, Version::new(2, 1, 0));
    }

    #[test]
    fn test_assemble_dependent_bump_none() {
        let ws = mock_workspace(vec![
            mock_package("a", "1.0.0", vec![]),
            mock_package("b", "2.0.0", vec!["a"]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "a".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config {
            dependent_bump: DependentBump::None,
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].name, "a");
    }

    #[test]
    fn test_assemble_fixed_group() {
        let ws = mock_workspace(vec![
            mock_package("x", "1.0.0", vec![]),
            mock_package("y", "1.0.0", vec![]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "x".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config {
            dependent_bump: DependentBump::None,
            fixed: vec![crate::config::FixedGroup {
                members: vec!["x".to_string(), "y".to_string()],
            }],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 2);
        let x = plan.releases.iter().find(|r| r.name == "x").unwrap();
        let y = plan.releases.iter().find(|r| r.name == "y").unwrap();
        assert_eq!(x.bump, BumpType::Minor);
        assert_eq!(y.bump, BumpType::Minor);
    }

    #[test]
    fn test_assemble_linked_group_both_releasing() {
        let ws = mock_workspace(vec![
            mock_package("p", "1.0.0", vec![]),
            mock_package("q", "1.0.0", vec![]),
        ]);
        let changelogs = vec![
            make_changelog(
                "cl1",
                vec![Release {
                    package: "p".to_string(),
                    bump: BumpType::Patch,
                }],
            ),
            make_changelog(
                "cl2",
                vec![Release {
                    package: "q".to_string(),
                    bump: BumpType::Minor,
                }],
            ),
        ];
        let config = Config {
            dependent_bump: DependentBump::None,
            linked: vec![crate::config::LinkedGroup {
                members: vec!["p".to_string(), "q".to_string()],
            }],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 2);
        let p = plan.releases.iter().find(|r| r.name == "p").unwrap();
        let q = plan.releases.iter().find(|r| r.name == "q").unwrap();
        assert_eq!(p.bump, BumpType::Minor);
        assert_eq!(q.bump, BumpType::Minor);
    }

    #[test]
    fn test_assemble_linked_group_only_one() {
        let ws = mock_workspace(vec![
            mock_package("p", "1.0.0", vec![]),
            mock_package("q", "1.0.0", vec![]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "p".to_string(),
                bump: BumpType::Patch,
            }],
        )];
        let config = Config {
            dependent_bump: DependentBump::None,
            linked: vec![crate::config::LinkedGroup {
                members: vec!["p".to_string(), "q".to_string()],
            }],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].name, "p");
        assert_eq!(plan.releases[0].bump, BumpType::Patch);
    }

    #[test]
    fn test_assemble_ignore_package() {
        let ws = mock_workspace(vec![
            mock_package("foo", "1.0.0", vec![]),
            mock_package("bar", "1.0.0", vec![]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![
                Release {
                    package: "foo".to_string(),
                    bump: BumpType::Minor,
                },
                Release {
                    package: "bar".to_string(),
                    bump: BumpType::Patch,
                },
            ],
        )];
        let config = Config {
            ignore: vec!["bar".to_string()],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].name, "foo");
        assert!(plan.releases.iter().all(|r| r.name != "bar"));
    }

    #[test]
    fn test_assemble_ignore_excludes_from_dependent_bump() {
        let ws = mock_workspace(vec![
            mock_package("a", "1.0.0", vec![]),
            mock_package("b", "2.0.0", vec!["a"]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "a".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config {
            ignore: vec!["b".to_string()],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 1);
        assert_eq!(plan.releases[0].name, "a");
        assert!(plan.releases.iter().all(|r| r.name != "b"));
    }

    #[test]
    fn test_prerelease_bump_table() {
        let rc = PrereleasePrefix::new("rc").unwrap();
        let cases = [
            ("1.3.0", BumpType::Minor, "1.4.0-rc1"),
            ("1.3.0-rc1", BumpType::Patch, "1.3.0-rc2"),
            ("1.3.0-rc1", BumpType::Minor, "1.3.0-rc2"),
            ("1.3.0-rc1", BumpType::Major, "2.0.0-rc1"),
            ("1.3.1-rc1", BumpType::Patch, "1.3.1-rc2"),
            ("1.3.1-rc1", BumpType::Minor, "1.4.0-rc1"),
            ("1.3.1-rc2", BumpType::Major, "2.0.0-rc1"),
            ("2.0.0-rc1", BumpType::Major, "2.0.0-rc2"),
            ("2.0.0-rc1", BumpType::Patch, "2.0.0-rc2"),
        ];

        for (current, bump, expected) in cases {
            let actual =
                bump_version_with_prerelease(&Version::parse(current).unwrap(), bump, Some(&rc));
            assert_eq!(
                actual,
                Version::parse(expected).unwrap(),
                "{current} + {bump} with rc"
            );
        }
    }

    #[test]
    fn test_stable_bump_on_prerelease_table() {
        let cases = [
            ("1.3.0-rc1", BumpType::Minor, "1.3.0"),
            ("1.3.0-rc1", BumpType::Major, "2.0.0"),
            ("1.3.1-rc1", BumpType::Minor, "1.4.0"),
            ("2.0.0-rc3", BumpType::Major, "2.0.0"),
        ];

        for (current, bump, expected) in cases {
            let actual =
                bump_version_with_prerelease(&Version::parse(current).unwrap(), bump, None);
            assert_eq!(
                actual,
                Version::parse(expected).unwrap(),
                "{current} + {bump}"
            );
        }
    }

    #[test]
    fn test_dependent_bump_into_fixed_group() {
        // core changes; app depends on core and is fixed with cli, so cli must
        // release with app even though nothing it depends on changed.
        let ws = mock_workspace(vec![
            mock_package("core", "1.0.0", vec![]),
            mock_package("app", "1.0.0", vec!["core"]),
            mock_package("cli", "1.0.0", vec![]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "core".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config {
            fixed: vec![crate::config::FixedGroup {
                members: vec!["app".to_string(), "cli".to_string()],
            }],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        let versions: Vec<_> = plan
            .releases
            .iter()
            .map(|r| (r.name.as_str(), r.new_version.to_string()))
            .collect();
        assert_eq!(
            versions,
            vec![
                ("app", "1.0.1".to_string()),
                ("cli", "1.0.1".to_string()),
                ("core", "1.1.0".to_string()),
            ]
        );
    }

    #[test]
    fn test_fixed_group_bump_triggers_dependents() {
        // a changes, fixed group pulls in b, and c depends on b.
        let ws = mock_workspace(vec![
            mock_package("a", "1.0.0", vec![]),
            mock_package("b", "1.0.0", vec![]),
            mock_package("c", "1.0.0", vec!["b"]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "a".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config {
            fixed: vec![crate::config::FixedGroup {
                members: vec!["a".to_string(), "b".to_string()],
            }],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        let c = plan.releases.iter().find(|r| r.name == "c").unwrap();
        assert_eq!(c.new_version, Version::new(1, 0, 1));
    }

    #[test]
    fn test_fixed_group_divergent_versions_share_highest() {
        let ws = mock_workspace(vec![
            mock_package("x", "1.0.0", vec![]),
            mock_package("y", "1.4.2", vec![]),
        ]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![Release {
                package: "x".to_string(),
                bump: BumpType::Minor,
            }],
        )];
        let config = Config {
            dependent_bump: DependentBump::None,
            fixed: vec![crate::config::FixedGroup {
                members: vec!["x".to_string(), "y".to_string()],
            }],
            ..Config::default()
        };

        let plan = assemble(&ws, changelogs, &config);

        assert_eq!(plan.releases.len(), 2);
        for release in &plan.releases {
            assert_eq!(
                release.new_version,
                Version::new(1, 5, 0),
                "{}",
                release.name
            );
        }
    }

    #[test]
    fn test_unknown_package_refs() {
        let ws = mock_workspace(vec![mock_package("foo", "1.0.0", vec![])]);
        let changelogs = vec![make_changelog(
            "cl1",
            vec![
                Release {
                    package: "foo".to_string(),
                    bump: BumpType::Patch,
                },
                Release {
                    package: "ghost".to_string(),
                    bump: BumpType::Patch,
                },
                Release {
                    package: "skipped".to_string(),
                    bump: BumpType::Patch,
                },
            ],
        )];
        let config = Config {
            ignore: vec!["skipped".to_string()],
            ..Config::default()
        };

        assert_eq!(
            unknown_package_refs(&ws, &changelogs, &config),
            vec![("cl1".to_string(), "ghost".to_string())]
        );
    }
}
