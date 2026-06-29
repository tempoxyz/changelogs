use changelogs::ecosystems::{self, Ecosystem};
use changelogs::{Package, PublishResult, SkipReason};
use semver::Version;
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

/// Python: absent `TWINE_PASSWORD`, even with Action's default `TWINE_USERNAME`,
/// returns `Skipped(NoToken)`, not an error.
#[test]
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

    // SAFETY: single-threaded test; no other test races on these env vars.
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

    // SAFETY: test-only cleanup.
    unsafe {
        std::env::remove_var("TWINE_USERNAME");
    }
}

#[test]
fn action_tags_only_mode_disables_registry_credentials() {
    let action = include_str!("../action.yml");

    assert!(
        action.contains(
            "if: steps.check.outputs.hasChangelogs == 'false' && inputs.ecosystem == 'python' && inputs.publish-mode == 'registry'"
        ),
        "PyPI auth must not run in tags-only mode"
    );

    for expected in [
        "PUBLISH_MODE: ${{ inputs.publish-mode }}",
        "if [ \"$PUBLISH_MODE\" = \"tags-only\" ]; then",
        "unset CARGO_REGISTRY_TOKEN",
        "unset TWINE_USERNAME",
        "unset TWINE_PASSWORD",
    ] {
        assert!(
            action.contains(expected),
            "tags-only mode must clear registry credential: {expected}"
        );
    }
}

#[test]
fn action_published_outputs_exclude_skipped_tag_only_rows() {
    let action = include_str!("../action.yml");
    let parser_line = action
        .lines()
        .find(|line| line.contains("published_packages=$(echo"))
        .expect("publish action should parse published package rows");

    assert!(
        parser_line.contains("grep -E \"✓\""),
        "publishedPackages must count only registry-published rows"
    );
    assert!(
        !parser_line.contains("⊘"),
        "publishedPackages must not count skipped/tag-only rows"
    );
}
