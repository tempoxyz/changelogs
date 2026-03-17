use changelogs::changelog_entry;
use changelogs::changelog_writer;
use changelogs::config::Config;
use changelogs::ecosystems::{Ecosystem, Package};
use changelogs::plan;
use changelogs::workspace::Workspace;
use semver::Version;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const TEST_DATE: &str = "2025-01-15";

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn should_bless() -> bool {
    std::env::var("UPDATE_GOLDENS").is_ok()
}

#[derive(Deserialize)]
struct PackageDef {
    name: String,
    version: String,
    #[serde(default)]
    deps: Vec<String>,
}

#[derive(Deserialize)]
struct PackagesManifest {
    packages: Vec<PackageDef>,
}

fn run_golden_test(fixture_name: &str) {
    let fixture = fixtures_root().join(fixture_name);
    let changelog_dir = fixture.join("changelog");

    if !changelog_dir.exists() {
        return;
    }

    let packages_toml = fixture.join("packages.toml");
    if !packages_toml.exists() {
        panic!("fixture {fixture_name} missing packages.toml");
    }

    let manifest: PackagesManifest =
        toml::from_str(&std::fs::read_to_string(&packages_toml).unwrap()).unwrap();

    let tmp = TempDir::new().unwrap();

    let packages: Vec<Package> = manifest
        .packages
        .iter()
        .map(|p| {
            let pkg_dir = tmp.path().join(&p.name);
            std::fs::create_dir_all(&pkg_dir).unwrap();
            Package {
                name: p.name.clone(),
                version: Version::parse(&p.version).unwrap(),
                path: pkg_dir.clone(),
                manifest_path: pkg_dir.join("Cargo.toml"),
                dependencies: p.deps.clone(),
            }
        })
        .collect();

    let workspace = Workspace {
        root: tmp.path().to_path_buf(),
        changelog_dir: changelog_dir.clone(),
        packages,
        ecosystem: Ecosystem::Rust,
    };

    let changelogs = changelog_entry::read_all(&changelog_dir).unwrap();

    let config = load_config(&fixture);
    let release_plan = plan::assemble(&workspace, changelogs.clone(), &config);

    let expected_dir = fixture.join("expected");
    if !expected_dir.exists() {
        std::fs::create_dir_all(&expected_dir).unwrap();
    }

    // Check releases.txt
    let releases_golden = expected_dir.join("releases.txt");
    if releases_golden.exists() || should_bless() {
        let actual = format_releases(&release_plan.releases);
        if should_bless() && !release_plan.releases.is_empty() {
            std::fs::write(&releases_golden, &actual).unwrap();
        } else if releases_golden.exists() {
            let expected = std::fs::read_to_string(&releases_golden).unwrap();
            assert_eq!(actual, expected, "[{fixture_name}] releases.txt mismatch");
        }
    }

    // Check CHANGELOG golden files
    let has_changelog_golden = std::fs::read_dir(&expected_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("CHANGELOG")
        });

    if !has_changelog_golden && !should_bless() {
        return;
    }

    changelog_writer::write_changelogs_with_date(
        &workspace,
        &release_plan.releases,
        &changelogs,
        config.changelog.format,
        TEST_DATE,
    )
    .unwrap();

    if should_bless() {
        bless_changelogs(&workspace, &config, &expected_dir);
        return;
    }

    for entry in std::fs::read_dir(&expected_dir).unwrap() {
        let entry = entry.unwrap();
        let filename = entry.file_name().to_string_lossy().to_string();

        if !filename.contains("CHANGELOG") {
            continue;
        }

        let expected = std::fs::read_to_string(entry.path()).unwrap();
        let actual_path = resolve_changelog_path(&workspace, &filename, &config);
        let actual = std::fs::read_to_string(&actual_path).unwrap_or_else(|_| {
            panic!(
                "[{fixture_name}] expected output file not produced: {}",
                actual_path.display()
            )
        });

        assert_eq!(actual, expected, "[{fixture_name}] {filename} mismatch");
    }
}

fn bless_changelogs(workspace: &Workspace, config: &Config, expected_dir: &Path) {
    use changelogs::config::ChangelogFormat;

    match config.changelog.format {
        ChangelogFormat::Root => {
            let src = workspace.root.join("CHANGELOG.md");
            if src.exists() {
                std::fs::copy(&src, expected_dir.join("CHANGELOG.md")).unwrap();
            }
        }
        ChangelogFormat::PerCrate => {
            if workspace.packages.len() == 1 {
                let src = workspace.packages[0].path.join("CHANGELOG.md");
                if src.exists() {
                    std::fs::copy(&src, expected_dir.join("CHANGELOG.md")).unwrap();
                }
            } else {
                for pkg in &workspace.packages {
                    let src = pkg.path.join("CHANGELOG.md");
                    if src.exists() {
                        let golden_name = format!("{}-CHANGELOG.md", pkg.name);
                        std::fs::copy(&src, expected_dir.join(golden_name)).unwrap();
                    }
                }
            }
        }
    }
}

fn resolve_changelog_path(workspace: &Workspace, golden_name: &str, config: &Config) -> PathBuf {
    use changelogs::config::ChangelogFormat;

    if config.changelog.format == ChangelogFormat::Root {
        return workspace.root.join("CHANGELOG.md");
    }

    // Per-crate: single package uses CHANGELOG.md directly
    if workspace.packages.len() == 1 && golden_name == "CHANGELOG.md" {
        return workspace.packages[0].path.join("CHANGELOG.md");
    }

    // Per-crate with multiple packages: golden file is named "<pkg>-CHANGELOG.md"
    if let Some(pkg_name) = golden_name.strip_suffix("-CHANGELOG.md") {
        if let Some(pkg) = workspace.packages.iter().find(|p| p.name == pkg_name) {
            return pkg.path.join("CHANGELOG.md");
        }
    }

    workspace.root.join(golden_name)
}

fn load_config(fixture_dir: &Path) -> Config {
    let config_path = fixture_dir.join("config.toml");
    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path).unwrap();
        toml::from_str(&content).unwrap()
    } else {
        Config::default()
    }
}

fn format_releases(releases: &[plan::PackageRelease]) -> String {
    releases
        .iter()
        .map(|r| {
            format!(
                "{}: {} -> {} ({})",
                r.name, r.old_version, r.new_version, r.bump
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

// ── Table-driven tests ──────────────────────────────────────────────

#[test]
fn golden_single_crate_patch() {
    run_golden_test("single-crate-patch");
}

#[test]
fn golden_multi_crate_mixed() {
    run_golden_test("multi-crate-mixed");
}

#[test]
fn golden_dependent_bump() {
    run_golden_test("dependent-bump");
}

#[test]
fn golden_fixed_groups() {
    run_golden_test("fixed-groups");
}

#[test]
fn golden_linked_groups() {
    run_golden_test("linked-groups");
}

#[test]
fn golden_ignore_packages() {
    run_golden_test("ignore-packages");
}

#[test]
fn golden_root_changelog() {
    run_golden_test("root-changelog");
}

#[test]
fn golden_multiple_changelogs_per_crate() {
    run_golden_test("multiple-changelogs-per-crate");
}

// ── Edge-case tests ─────────────────────────────────────────────────

#[test]
fn golden_empty_changelog() {
    run_golden_test("empty-changelog");
}

#[test]
fn golden_all_ignored() {
    run_golden_test("all-ignored");
}

#[test]
fn golden_cyclic_deps() {
    run_golden_test("cyclic-deps");
}

#[test]
fn golden_fixed_and_linked() {
    run_golden_test("fixed-and-linked");
}

#[test]
fn golden_major_bump() {
    run_golden_test("major-bump");
}

#[test]
fn golden_ignore_with_dependent_bump() {
    run_golden_test("ignore-with-dependent-bump");
}

#[test]
fn golden_multi_crate_multi_changelog() {
    run_golden_test("multi-crate-multi-changelog");
}

#[test]
fn golden_invalid_frontmatter() {
    let fixture = fixtures_root().join("invalid-frontmatter");
    let changelog_dir = fixture.join("changelog");
    let result = changelog_entry::read_all(&changelog_dir);
    assert!(
        result.is_err(),
        "expected parse error for invalid frontmatter"
    );
}
