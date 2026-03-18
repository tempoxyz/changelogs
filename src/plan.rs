use crate::BumpType;
use crate::changelog_entry::Changelog;
use crate::config::{ChangelogFormat, Config, DependentBump};
use crate::graph::DependencyGraph;
use crate::workspace::Workspace;
use semver::Version;
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

pub fn bump_version(version: &Version, bump: BumpType) -> Version {
    match bump {
        BumpType::Major => Version::new(version.major + 1, 0, 0),
        BumpType::Minor => Version::new(version.major, version.minor + 1, 0),
        BumpType::Patch => Version::new(version.major, version.minor, version.patch + 1),
    }
}

pub fn assemble(workspace: &Workspace, changelogs: Vec<Changelog>, config: &Config) -> ReleasePlan {
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

    if config.dependent_bump != DependentBump::None {
        let dependent_bump_type = match config.dependent_bump {
            DependentBump::Patch => BumpType::Patch,
            DependentBump::Minor => BumpType::Minor,
            DependentBump::None => unreachable!(),
        };

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

    let mut warnings: Vec<String> = Vec::new();
    let mut releases: Vec<PackageRelease> = Vec::new();

    for (name, bump) in bump_map {
        if let Some(package) = workspace.get_package(&name) {
            let new_version = bump_version(&package.version, bump);
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
        let changelogs = vec![
            Changelog {
                id: "a".to_string(),
                summary: "patch change".to_string(),
                releases: vec![Release {
                    package: "foo".to_string(),
                    bump: BumpType::Patch,
                }],
                commit: None,
            },
            Changelog {
                id: "b".to_string(),
                summary: "minor change".to_string(),
                releases: vec![Release {
                    package: "foo".to_string(),
                    bump: BumpType::Minor,
                }],
                commit: None,
            },
        ];

        let _config = Config::default();

        let bump_map: HashMap<String, BumpType> = changelogs
            .iter()
            .flat_map(|cs| &cs.releases)
            .fold(HashMap::new(), |mut acc, rel| {
                let current = acc.get(&rel.package).copied();
                let new_bump = match current {
                    Some(existing) => existing.max(rel.bump),
                    None => rel.bump,
                };
                acc.insert(rel.package.clone(), new_bump);
                acc
            });

        assert_eq!(bump_map.get("foo"), Some(&BumpType::Minor));
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
}
