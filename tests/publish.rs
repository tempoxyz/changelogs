use changelogs::ecosystems::{self, Ecosystem};
use changelogs::{Package, PublishResult, SkipReason};
use semver::Version;
use serial_test::serial;
use tempfile::TempDir;

fn rust_package(dir: &std::path::Path) -> Package {
    let manifest = dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"my-binary\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    Package {
        name: "my-binary".to_string(),
        version: Version::new(1, 0, 0),
        path: dir.to_path_buf(),
        manifest_path: manifest,
        dependencies: vec![],
    }
}

/// Rust: absent `CARGO_REGISTRY_TOKEN` → `Skipped(NoToken)`, not an error.
///
/// Binary-only projects (e.g. foundry-rs/foundry) skip crates.io but still
/// want git tags and GitHub releases. The publish step must succeed so the
/// downstream tagging steps can run.
#[test]
#[serial]
fn rust_publish_without_token_skips_silently() {
    let dir = TempDir::new().unwrap();
    let pkg = rust_package(dir.path());

    // SAFETY: single-threaded test; no other test races on this env var.
    unsafe {
        std::env::remove_var("CARGO_REGISTRY_TOKEN");
    }

    let result = ecosystems::publish(Ecosystem::Rust, &pkg, false, None)
        .expect("publish must not error when no registry token is configured");

    assert_eq!(
        result,
        PublishResult::Skipped(SkipReason::NoToken),
        "expected Skipped(NoToken) — git-tag creation must still proceed"
    );
}

/// Python: absent `TWINE_PASSWORD` → `Skipped(NoToken)`, not an error.
///
/// The action always sets `TWINE_USERNAME=__token__`, so the no-token
/// condition must be triggered by the absence of `TWINE_PASSWORD` alone.
#[test]
#[serial]
fn python_publish_without_token_skips_silently() {
    let dir = TempDir::new().unwrap();
    let manifest = dir.path().join("pyproject.toml");
    std::fs::write(
        &manifest,
        "[project]\nname = \"my-tool\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let pkg = Package {
        name: "my-tool".to_string(),
        version: Version::new(1, 0, 0),
        path: dir.path().to_path_buf(),
        manifest_path: manifest,
        dependencies: vec![],
    };

    // Mirror the action's real "no pypi-token" environment: TWINE_USERNAME is
    // always set to __token__ by the action; TWINE_PASSWORD is what's absent.
    // SAFETY: serialized via #[serial]; no concurrent env mutation.
    unsafe {
        std::env::remove_var("TWINE_PASSWORD");
        std::env::set_var("TWINE_USERNAME", "__token__");
    }

    let result = ecosystems::publish(Ecosystem::Python, &pkg, false, None)
        .expect("publish must not error when no registry token is configured");

    assert_eq!(
        result,
        PublishResult::Skipped(SkipReason::NoToken),
        "expected Skipped(NoToken) — git-tag creation must still proceed"
    );
}
