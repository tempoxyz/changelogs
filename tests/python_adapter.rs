use changelogs::ecosystems::{Ecosystem, EcosystemAdapter, PythonAdapter};
use semver::Version;
use std::path::PathBuf;
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn create_pyproject(dir: &std::path::Path, content: &str) {
    std::fs::write(dir.join("pyproject.toml"), content).unwrap();
}

#[test]
fn test_python_discover() {
    let root = fixture_path("python-simple");
    let packages = PythonAdapter::discover(&root, false).unwrap();

    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "my-package");
    assert_eq!(packages[0].version, Version::new(0, 1, 0));
    assert!(packages[0].manifest_path.ends_with("pyproject.toml"));
}

#[test]
fn test_python_read_version() {
    let root = fixture_path("python-simple");
    let manifest_path = root.join("pyproject.toml");

    let version = PythonAdapter::read_version(&manifest_path).unwrap();
    assert_eq!(version, Version::new(0, 1, 0));
}

#[test]
fn test_python_write_version() {
    let temp_dir = TempDir::new().unwrap();
    let pyproject_path = temp_dir.path().join("pyproject.toml");

    let original_content = r#"[project]
name = "test-package"
version = "1.0.0"
description = "Test"

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
"#;
    std::fs::write(&pyproject_path, original_content).unwrap();

    let new_version = Version::new(2, 0, 0);
    PythonAdapter::write_version(&pyproject_path, &new_version).unwrap();

    let updated_content = std::fs::read_to_string(&pyproject_path).unwrap();
    assert!(updated_content.contains("version = \"2.0.0\""));
    assert!(updated_content.contains("name = \"test-package\""));
    assert!(updated_content.contains("[build-system]"));
}

#[test]
fn test_python_discover_rejects_dynamic_version() {
    let temp_dir = TempDir::new().unwrap();
    let pyproject_path = temp_dir.path().join("pyproject.toml");

    let content = r#"[project]
name = "dynamic-package"
dynamic = ["version"]

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
"#;
    std::fs::write(&pyproject_path, content).unwrap();

    let result = PythonAdapter::discover(temp_dir.path(), false);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Dynamic versions"));
}

#[test]
fn test_python_discover_requires_project_section() {
    let temp_dir = TempDir::new().unwrap();
    let pyproject_path = temp_dir.path().join("pyproject.toml");

    let content = r#"[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
"#;
    std::fs::write(&pyproject_path, content).unwrap();

    let result = PythonAdapter::discover(temp_dir.path(), false);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("[project]"));
}

#[test]
fn test_ecosystem_detection() {
    let rust_dir = TempDir::new().unwrap();
    std::fs::write(rust_dir.path().join("Cargo.toml"), "[package]").unwrap();

    let python_dir = TempDir::new().unwrap();
    std::fs::write(python_dir.path().join("pyproject.toml"), "[project]").unwrap();

    let empty_dir = TempDir::new().unwrap();

    assert_eq!(
        changelogs::ecosystems::detect_ecosystem(rust_dir.path()),
        Some(Ecosystem::Rust)
    );
    assert_eq!(
        changelogs::ecosystems::detect_ecosystem(python_dir.path()),
        Some(Ecosystem::Python)
    );
    assert_eq!(
        changelogs::ecosystems::detect_ecosystem(empty_dir.path()),
        None
    );
}

#[test]
fn test_ecosystem_detection_from_subdirectory() {
    let temp_dir = TempDir::new().unwrap();
    let subdir = temp_dir.path().join("src").join("nested");
    std::fs::create_dir_all(&subdir).unwrap();

    create_pyproject(
        temp_dir.path(),
        r#"[project]
name = "parent-pkg"
version = "1.0.0"
"#,
    );

    assert_eq!(
        changelogs::ecosystems::detect_ecosystem(&subdir),
        Some(Ecosystem::Python)
    );
}

#[test]
fn test_update_dependency_preserves_extras() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[project]
name = "test-pkg"
version = "1.0.0"
dependencies = [
    "requests[socks]>=2.0",
]
"#,
    );

    let manifest_path = temp_dir.path().join("pyproject.toml");
    let new_version = Version::new(3, 0, 0);
    let modified =
        PythonAdapter::update_dependency_version(&manifest_path, "requests", &new_version).unwrap();

    assert!(modified);
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("requests[socks]==3.0.0"));
}

#[test]
fn test_update_dependency_preserves_markers() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[project]
name = "test-pkg"
version = "1.0.0"
dependencies = [
    "typing-extensions>=4.0; python_version < '3.11'",
]
"#,
    );

    let manifest_path = temp_dir.path().join("pyproject.toml");
    let new_version = Version::new(5, 0, 0);
    let modified =
        PythonAdapter::update_dependency_version(&manifest_path, "typing-extensions", &new_version)
            .unwrap();

    assert!(modified);
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("typing-extensions==5.0.0; python_version < '3.11'"));
}

#[test]
fn test_update_dependency_preserves_extras_and_markers() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[project]
name = "test-pkg"
version = "1.0.0"
dependencies = [
    "httpx[http2]>=0.20; sys_platform != 'win32'",
]
"#,
    );

    let manifest_path = temp_dir.path().join("pyproject.toml");
    let new_version = Version::new(1, 0, 0);
    let modified =
        PythonAdapter::update_dependency_version(&manifest_path, "httpx", &new_version).unwrap();

    assert!(modified);
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("httpx[http2]==1.0.0; sys_platform != 'win32'"));
}

#[test]
fn test_pep503_name_normalization() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[project]
name = "test-pkg"
version = "1.0.0"
dependencies = [
    "Foo_Bar>=1.0",
    "foo.baz>=2.0",
    "foo---qux>=3.0",
]
"#,
    );

    let packages = PythonAdapter::discover(temp_dir.path(), false).unwrap();
    let deps = &packages[0].dependencies;

    assert!(deps.contains(&"foo-bar".to_string()));
    assert!(deps.contains(&"foo-baz".to_string()));
    assert!(deps.contains(&"foo-qux".to_string()));
}

#[test]
fn test_update_dependency_in_optional_deps() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[project]
name = "test-pkg"
version = "1.0.0"
dependencies = []

[project.optional-dependencies]
dev = [
    "pytest[cov]>=7.0",
]
"#,
    );

    let manifest_path = temp_dir.path().join("pyproject.toml");
    let new_version = Version::new(8, 0, 0);
    let modified =
        PythonAdapter::update_dependency_version(&manifest_path, "pytest", &new_version).unwrap();

    assert!(modified);
    let content = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("pytest[cov]==8.0.0"));
}

#[test]
fn test_poetry_discover() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[tool.poetry]
name = "poetry-package"
version = "1.2.3"
description = "A Poetry project"

[tool.poetry.dependencies]
python = "^3.8"
requests = "^2.28"
click = "^8.0"

[tool.poetry.group.dev.dependencies]
pytest = "^7.0"
"#,
    );

    let packages = PythonAdapter::discover(temp_dir.path(), false).unwrap();

    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "poetry-package");
    assert_eq!(packages[0].version, Version::new(1, 2, 3));
    assert!(packages[0].dependencies.contains(&"requests".to_string()));
    assert!(packages[0].dependencies.contains(&"click".to_string()));
    assert!(packages[0].dependencies.contains(&"pytest".to_string()));
    assert!(!packages[0].dependencies.contains(&"python".to_string()));
}

#[test]
fn test_poetry_read_write_version() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[tool.poetry]
name = "poetry-package"
version = "1.0.0"
description = "Test"

[tool.poetry.dependencies]
python = "^3.8"
"#,
    );

    let manifest_path = temp_dir.path().join("pyproject.toml");

    let version = PythonAdapter::read_version(&manifest_path).unwrap();
    assert_eq!(version, Version::new(1, 0, 0));

    let new_version = Version::new(2, 0, 0);
    PythonAdapter::write_version(&manifest_path, &new_version).unwrap();

    let updated_version = PythonAdapter::read_version(&manifest_path).unwrap();
    assert_eq!(updated_version, Version::new(2, 0, 0));

    let content = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(content.contains("[tool.poetry]"));
    assert!(content.contains("version = \"2.0.0\""));
}

#[test]
fn test_pep621_takes_precedence_over_poetry() {
    let temp_dir = TempDir::new().unwrap();
    create_pyproject(
        temp_dir.path(),
        r#"[project]
name = "pep621-package"
version = "1.0.0"

[tool.poetry]
name = "poetry-package"
version = "2.0.0"
"#,
    );

    let packages = PythonAdapter::discover(temp_dir.path(), false).unwrap();

    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "pep621-package");
    assert_eq!(packages[0].version, Version::new(1, 0, 0));
}
