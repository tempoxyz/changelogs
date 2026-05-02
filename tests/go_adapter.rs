use changelogs::ecosystems::{Ecosystem, EcosystemAdapter, GoAdapter};
use semver::Version;
use std::path::PathBuf;
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_go_discover_fixture() {
    let root = fixture_path("go-simple");
    let packages = GoAdapter::discover(&root).unwrap();

    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "widget");
    assert_eq!(packages[0].version, Version::new(0, 3, 1));
    assert!(packages[0].manifest_path.ends_with("go.mod"));
    // Both required modules surface as dependencies (block form).
    assert!(
        packages[0]
            .dependencies
            .iter()
            .any(|d| d == "github.com/spf13/cobra")
    );
    assert!(
        packages[0]
            .dependencies
            .iter()
            .any(|d| d == "github.com/stretchr/testify")
    );
}

#[test]
fn test_go_read_version_from_fixture() {
    let manifest = fixture_path("go-simple").join("go.mod");
    let version = GoAdapter::read_version(&manifest).unwrap();
    assert_eq!(version, Version::new(0, 3, 1));
}

#[test]
fn test_go_write_version_round_trip() {
    let tmp = TempDir::new().unwrap();
    let manifest = tmp.path().join("go.mod");
    std::fs::write(
        &manifest,
        "module github.com/foo/widget\n\ngo 1.22\n\nrequire github.com/a/b v1.0.0\n",
    )
    .unwrap();

    let v0 = GoAdapter::read_version(&manifest).unwrap();
    assert_eq!(v0, Version::new(0, 0, 0));

    GoAdapter::write_version(&manifest, &Version::new(0, 5, 0)).unwrap();
    let v1 = GoAdapter::read_version(&manifest).unwrap();
    assert_eq!(v1, Version::new(0, 5, 0));

    let on_disk = std::fs::read_to_string(&manifest).unwrap();
    assert!(on_disk.contains("// changelogs:version 0.5.0"));
    // Original lines preserved.
    assert!(on_disk.contains("module github.com/foo/widget"));
    assert!(on_disk.contains("go 1.22"));
    assert!(on_disk.contains("require github.com/a/b v1.0.0"));
}

#[test]
fn test_go_update_dependency_version_block_form() {
    let tmp = TempDir::new().unwrap();
    let manifest = tmp.path().join("go.mod");
    std::fs::write(
        &manifest,
        "module github.com/foo/widget\n\nrequire (\n\tgithub.com/a/b v1.0.0\n\tgithub.com/c/d v2.1.0 // indirect\n)\n",
    )
    .unwrap();

    let modified =
        GoAdapter::update_dependency_version(&manifest, "github.com/a/b", &Version::new(1, 5, 0))
            .unwrap();
    assert!(modified);

    let content = std::fs::read_to_string(&manifest).unwrap();
    assert!(content.contains("github.com/a/b v1.5.0"));
    // Indirect dep is untouched and keeps its trailing comment.
    assert!(content.contains("github.com/c/d v2.1.0 // indirect"));
}

#[test]
fn test_go_tag_name_v_prefixed() {
    let root = fixture_path("go-simple");
    let pkg = &GoAdapter::discover(&root).unwrap()[0];
    assert_eq!(GoAdapter::tag_name(pkg), "v0.3.1");
}

#[test]
fn test_go_ecosystem_detection() {
    let go_dir = TempDir::new().unwrap();
    std::fs::write(
        go_dir.path().join("go.mod"),
        "module github.com/foo/bar\n\ngo 1.22\n",
    )
    .unwrap();

    assert_eq!(
        changelogs::ecosystems::detect_ecosystem(go_dir.path()),
        Some(Ecosystem::Go)
    );
}

#[test]
fn test_go_ecosystem_detection_from_subdirectory() {
    let temp_dir = TempDir::new().unwrap();
    std::fs::write(
        temp_dir.path().join("go.mod"),
        "module github.com/foo/bar\n\ngo 1.22\n",
    )
    .unwrap();

    let subdir = temp_dir.path().join("internal").join("nested");
    std::fs::create_dir_all(&subdir).unwrap();

    assert_eq!(
        changelogs::ecosystems::detect_ecosystem(&subdir),
        Some(Ecosystem::Go)
    );
}

#[test]
fn test_go_alias_parsing() {
    use std::str::FromStr;
    assert_eq!(Ecosystem::from_str("go").unwrap(), Ecosystem::Go);
    assert_eq!(Ecosystem::from_str("golang").unwrap(), Ecosystem::Go);
    assert_eq!(Ecosystem::from_str("GO").unwrap(), Ecosystem::Go);
}
