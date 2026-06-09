use changelogs::ecosystems::{Ecosystem, EcosystemAdapter, SwiftAdapter};
use semver::Version;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_swift_discover_fixture() {
    let root = fixture_path("swift-simple");
    let packages = SwiftAdapter::discover(&root).unwrap();

    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "TempoKit");
    assert_eq!(packages[0].version, Version::new(0, 4, 2));
    assert!(packages[0].manifest_path.ends_with("Package.swift"));
    assert!(packages[0].dependencies.contains(&"swift-log".to_string()));
    assert!(
        packages[0]
            .dependencies
            .contains(&"swift-collections".to_string())
    );
}

#[test]
fn test_swift_read_version_from_fixture() {
    let manifest = fixture_path("swift-simple").join("Package.swift");
    let version = SwiftAdapter::read_version(&manifest).unwrap();
    assert_eq!(version, Version::new(0, 4, 2));
}

#[test]
fn test_swift_write_version_round_trip() {
    let tmp = TempDir::new().unwrap();
    let manifest = tmp.path().join("Package.swift");
    std::fs::write(
        &manifest,
        r#"// swift-tools-version: 5.10
import PackageDescription

let package = Package(name: "TempoKit")
"#,
    )
    .unwrap();

    let v0 = SwiftAdapter::read_version(&manifest).unwrap();
    assert_eq!(v0, Version::new(0, 0, 0));

    SwiftAdapter::write_version(&manifest, &Version::new(0, 5, 0)).unwrap();
    let v1 = SwiftAdapter::read_version(&manifest).unwrap();
    assert_eq!(v1, Version::new(0, 5, 0));

    let on_disk = std::fs::read_to_string(&manifest).unwrap();
    assert!(on_disk.contains("// changelogs:version 0.5.0"));
    assert!(on_disk.contains("// swift-tools-version: 5.10"));
    assert!(on_disk.contains(r#"Package(name: "TempoKit")"#));
}

#[test]
fn test_swift_update_dependency_version() {
    let tmp = TempDir::new().unwrap();
    let manifest = tmp.path().join("Package.swift");
    std::fs::write(
        &manifest,
        r#"// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "TempoKit",
    dependencies: [
        .package(url: "https://github.com/apple/swift-log.git", from: "1.5.0"),
        .package(url: "https://github.com/apple/swift-collections.git", exact: "1.1.0"),
    ]
)
"#,
    )
    .unwrap();

    let modified =
        SwiftAdapter::update_dependency_version(&manifest, "swift-log", &Version::new(1, 6, 0))
            .unwrap();
    assert!(modified);

    let content = std::fs::read_to_string(&manifest).unwrap();
    assert!(content.contains(r#"swift-log.git", from: "1.6.0""#));
    assert!(content.contains(r#"swift-collections.git", exact: "1.1.0""#));
}

#[test]
fn test_swift_tag_name_v_prefixed() {
    let root = fixture_path("swift-simple");
    let pkg = &SwiftAdapter::discover(&root).unwrap()[0];
    assert_eq!(SwiftAdapter::tag_name(pkg), "v0.4.2");
}

#[test]
fn test_swift_ecosystem_detection() {
    let swift_dir = TempDir::new().unwrap();
    std::fs::write(
        swift_dir.path().join("Package.swift"),
        r#"// swift-tools-version: 5.10
import PackageDescription
let package = Package(name: "TempoKit")
"#,
    )
    .unwrap();

    assert_eq!(
        changelogs::ecosystems::detect_ecosystem(swift_dir.path()),
        Some(Ecosystem::Swift)
    );
}

#[test]
fn test_swift_alias_parsing() {
    use std::str::FromStr;
    assert_eq!(Ecosystem::from_str("swift").unwrap(), Ecosystem::Swift);
    assert_eq!(Ecosystem::from_str("swiftpm").unwrap(), Ecosystem::Swift);
    assert_eq!(Ecosystem::from_str("SPM").unwrap(), Ecosystem::Swift);
}

#[test]
fn test_swift_is_published_checks_existing_git_tag() {
    let tmp = TempDir::new().unwrap();
    let previous_dir = std::env::current_dir().unwrap();

    run_git(tmp.path(), &["init"]);
    run_git(tmp.path(), &["config", "user.email", "test@example.com"]);
    run_git(tmp.path(), &["config", "user.name", "Test User"]);
    std::fs::write(tmp.path().join("README.md"), "test\n").unwrap();
    run_git(tmp.path(), &["add", "README.md"]);
    run_git(tmp.path(), &["commit", "-m", "init"]);
    run_git(tmp.path(), &["tag", "v0.4.2"]);

    std::env::set_current_dir(tmp.path()).unwrap();
    assert!(SwiftAdapter::is_published("TempoKit", &Version::new(0, 4, 2)).unwrap());
    assert!(!SwiftAdapter::is_published("TempoKit", &Version::new(0, 4, 3)).unwrap());
    std::env::set_current_dir(previous_dir).unwrap();
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
