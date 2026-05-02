use crate::ecosystems::{Ecosystem, EcosystemAdapter, Package, PublishResult, SkipReason};
use crate::error::Result;
use cargo_metadata::MetadataCommand;
use semver::Version;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use toml_edit::DocumentMut;

pub struct RustAdapter;

impl EcosystemAdapter for RustAdapter {
    fn ecosystem() -> Ecosystem {
        Ecosystem::Rust
    }

    fn discover(root: &Path) -> Result<Vec<Package>> {
        let metadata = MetadataCommand::new().current_dir(root).exec()?;

        let workspace_members: std::collections::HashSet<_> =
            metadata.workspace_members.iter().collect();

        let mut packages = Vec::new();

        for package in &metadata.packages {
            if !workspace_members.contains(&package.id) {
                continue;
            }

            let deps: Vec<String> = package
                .dependencies
                .iter()
                .filter_map(|dep| {
                    metadata
                        .packages
                        .iter()
                        .find(|p| p.name.as_str() == dep.name && workspace_members.contains(&p.id))
                        .map(|p| p.name.to_string())
                })
                .collect();

            packages.push(Package {
                name: package.name.to_string(),
                version: package.version.clone(),
                path: package
                    .manifest_path
                    .parent()
                    .unwrap()
                    .to_path_buf()
                    .into_std_path_buf(),
                manifest_path: package.manifest_path.clone().into_std_path_buf(),
                dependencies: deps,
            });
        }

        Ok(packages)
    }

    fn read_version(manifest_path: &Path) -> Result<Version> {
        let content = std::fs::read_to_string(manifest_path)?;
        let doc: DocumentMut = content.parse()?;

        // Check if version uses workspace inheritance (version.workspace = true)
        if Self::is_workspace_inherited(&doc, "version") {
            let root = Self::find_workspace_root(manifest_path)?;
            let root_manifest = root.join("Cargo.toml");
            let root_content = std::fs::read_to_string(&root_manifest)?;
            let root_doc: DocumentMut = root_content.parse()?;

            let version_str = root_doc
                .get("workspace")
                .and_then(|w| w.get("package"))
                .and_then(|p| p.get("version"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    crate::error::Error::VersionNotFound(root_manifest.display().to_string())
                })?;

            return Ok(version_str.parse()?);
        }

        let version_str = doc["package"]["version"].as_str().ok_or_else(|| {
            crate::error::Error::VersionNotFound(manifest_path.display().to_string())
        })?;

        Ok(version_str.parse()?)
    }

    fn write_version(manifest_path: &Path, version: &Version) -> Result<()> {
        let content = std::fs::read_to_string(manifest_path)?;
        let doc: DocumentMut = content.parse()?;

        // If version is inherited from workspace, write to root Cargo.toml instead
        if Self::is_workspace_inherited(&doc, "version") {
            let root = Self::find_workspace_root(manifest_path)?;
            let root_manifest = root.join("Cargo.toml");
            let root_content = std::fs::read_to_string(&root_manifest)?;
            let mut root_doc: DocumentMut = root_content.parse()?;

            root_doc["workspace"]["package"]["version"] = toml_edit::value(version.to_string());

            std::fs::write(&root_manifest, root_doc.to_string())?;
            return Ok(());
        }

        let mut doc: DocumentMut = content.parse()?;
        doc["package"]["version"] = toml_edit::value(version.to_string());

        std::fs::write(manifest_path, doc.to_string())?;
        Ok(())
    }

    fn update_dependency_version(
        manifest_path: &Path,
        dep_name: &str,
        new_version: &Version,
    ) -> Result<bool> {
        let content = std::fs::read_to_string(manifest_path)?;
        let mut doc: DocumentMut = content.parse()?;
        let mut modified = false;

        for section in &["dependencies", "dev-dependencies", "build-dependencies"] {
            let Some(dep) = doc.get_mut(section).and_then(|d| d.get_mut(dep_name)) else {
                continue;
            };
            modified |= Self::update_dep_version_in_item(dep, new_version);
        }

        if let Some(dep) = doc
            .get_mut("workspace")
            .and_then(|w| w.get_mut("dependencies"))
            .and_then(|d| d.get_mut(dep_name))
        {
            modified |= Self::update_dep_version_in_item(dep, new_version);
        }

        if modified {
            std::fs::write(manifest_path, doc.to_string())?;
        }

        Ok(modified)
    }

    fn is_published(name: &str, version: &Version) -> Result<bool> {
        let output = Command::new("cargo")
            .args(["search", "--limit", "1", name])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        let is_published_with_same_version = stdout
            .lines()
            .next()
            .map(|line| line.contains(&format!("\"{}\"", version)))
            .unwrap_or(false);

        Ok(is_published_with_same_version)
    }

    fn publish(pkg: &Package, dry_run: bool, registry: Option<&str>) -> Result<PublishResult> {
        if dry_run {
            return Ok(PublishResult::Success);
        }

        // Skip crates that have `publish = false` in their Cargo.toml
        let content = std::fs::read_to_string(&pkg.manifest_path)?;
        let doc: DocumentMut = content.parse()?;
        if let Some(publish) = doc
            .get("package")
            .and_then(|p| p.get("publish"))
            .and_then(|v| v.as_bool())
        {
            if !publish {
                return Ok(PublishResult::Skipped(SkipReason::NotPublishable));
            }
        }

        match std::env::var("CARGO_REGISTRY_TOKEN") {
            Ok(token) if !token.is_empty() => {}
            _ => return Ok(PublishResult::Skipped(SkipReason::NoToken)),
        }

        let mut cmd = Command::new("cargo");
        cmd.arg("publish")
            .arg("--package")
            .arg(&pkg.name)
            .arg("--no-verify")
            .arg("--allow-dirty");

        if let Some(reg) = registry {
            cmd.env("CARGO_REGISTRY_DEFAULT", reg);
        }

        let output = cmd.output().map_err(|e| {
            crate::error::Error::PublishFailed(format!("failed to run 'cargo publish': {}", e))
        })?;

        if output.status.success() {
            return Ok(PublishResult::Success);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already uploaded") || stderr.contains("already exists") {
            return Ok(PublishResult::Success);
        }

        Err(crate::error::Error::PublishFailed(format!(
            "cargo publish failed (exit code {}):\nstdout: {}\nstderr: {}",
            output.status,
            stdout.trim(),
            stderr.trim(),
        )))
    }
}

impl RustAdapter {
    /// Check if a field in `[package]` uses workspace inheritance (e.g., `version.workspace = true`).
    fn is_workspace_inherited(doc: &DocumentMut, field: &str) -> bool {
        doc.get("package")
            .and_then(|p| p.get(field))
            .and_then(|v| v.as_table_like())
            .and_then(|t| t.get("workspace"))
            .and_then(|w| w.as_bool())
            .unwrap_or(false)
    }

    /// Walk up from a crate's manifest to find the workspace root containing `[workspace]`.
    fn find_workspace_root(manifest_path: &Path) -> Result<std::path::PathBuf> {
        let mut current = manifest_path
            .parent()
            .ok_or_else(|| crate::error::Error::VersionNotFound("no parent directory".to_string()))?
            .to_path_buf();

        loop {
            let candidate = current.join("Cargo.toml");
            if candidate.exists() && candidate != manifest_path {
                let content = std::fs::read_to_string(&candidate)?;
                let doc: DocumentMut = content.parse()?;
                if doc.get("workspace").is_some() {
                    return Ok(current);
                }
            }

            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => {
                    return Err(crate::error::Error::VersionNotFound(
                        "workspace root not found".to_string(),
                    ));
                }
            }
        }
    }

    fn update_dep_version_in_item(dep: &mut toml_edit::Item, new_version: &Version) -> bool {
        if let Some(table) = dep.as_inline_table_mut() {
            if table.contains_key("version") {
                table.insert("version", new_version.to_string().into());
                return true;
            }
        } else if let Some(table) = dep.as_table_mut() {
            if table.contains_key("version") {
                table["version"] = toml_edit::value(new_version.to_string());
                return true;
            }
        }
        false
    }

    pub fn update_all_dependency_versions(
        packages: &[Package],
        root: &Path,
        updates: &HashMap<String, Version>,
    ) -> Result<()> {
        for package in packages {
            for (dep_name, new_version) in updates {
                Self::update_dependency_version(&package.manifest_path, dep_name, new_version)?;
            }
        }

        let root_manifest = root.join("Cargo.toml");
        if !root_manifest.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&root_manifest)?;
        let mut doc: DocumentMut = content.parse()?;
        let mut modified = false;

        if let Some(deps) = doc
            .get_mut("workspace")
            .and_then(|w| w.get_mut("dependencies"))
            .and_then(|d| d.as_table_mut())
        {
            for (dep_name, new_version) in updates {
                if let Some(dep) = deps.get_mut(dep_name) {
                    modified |= Self::update_dep_version_in_item(dep, new_version);
                }
            }
        }

        if modified {
            std::fs::write(&root_manifest, doc.to_string())?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_version() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"test\"\nversion = \"1.2.3\"\n",
        )
        .unwrap();

        let version = RustAdapter::read_version(&manifest).unwrap();
        assert_eq!(version, Version::new(1, 2, 3));
    }

    #[test]
    #[should_panic(expected = "index not found")]
    fn test_read_version_missing_version() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nname = \"test\"\n").unwrap();

        let _ = RustAdapter::read_version(&manifest);
    }

    #[test]
    fn test_write_version() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"test\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        RustAdapter::write_version(&manifest, &Version::new(2, 3, 4)).unwrap();

        let version = RustAdapter::read_version(&manifest).unwrap();
        assert_eq!(version, Version::new(2, 3, 4));
    }

    #[test]
    fn test_write_version_preserves_other_fields() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        let content = "\
[package]\n\
name = \"test\"\n\
version = \"1.0.0\"\n\
edition = \"2021\"\n\
\n\
[dependencies]\n\
serde = \"1\"\n";
        std::fs::write(&manifest, content).unwrap();

        RustAdapter::write_version(&manifest, &Version::new(2, 0, 0)).unwrap();

        let updated = std::fs::read_to_string(&manifest).unwrap();
        assert!(updated.contains("name = \"test\""));
        assert!(updated.contains("edition = \"2021\""));
        assert!(updated.contains("serde = \"1\""));
        assert!(updated.contains("version = \"2.0.0\""));
    }

    #[test]
    fn test_update_dependency_version_regular_table() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        let content = "\
[package]\n\
name = \"test\"\n\
version = \"1.0.0\"\n\
\n\
[dependencies]\n\
my-dep = { version = \"1.0.0\", features = [\"serde\"] }\n";
        std::fs::write(&manifest, content).unwrap();

        let modified =
            RustAdapter::update_dependency_version(&manifest, "my-dep", &Version::new(2, 0, 0))
                .unwrap();
        assert!(modified);

        let updated = std::fs::read_to_string(&manifest).unwrap();
        assert!(updated.contains("version = \"2.0.0\""));
        assert!(updated.contains("serde"));
    }

    #[test]
    fn test_update_dependency_version_not_found() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        let content = "\
[package]\n\
name = \"test\"\n\
version = \"1.0.0\"\n\
\n\
[dependencies]\n\
other-dep = \"1.0\"\n";
        std::fs::write(&manifest, content).unwrap();

        let modified =
            RustAdapter::update_dependency_version(&manifest, "my-dep", &Version::new(2, 0, 0))
                .unwrap();
        assert!(!modified);
    }

    #[test]
    fn test_update_dependency_in_dev_deps() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        let content = "\
[package]\n\
name = \"test\"\n\
version = \"1.0.0\"\n\
\n\
[dev-dependencies]\n\
my-dep = { version = \"1.0.0\" }\n";
        std::fs::write(&manifest, content).unwrap();

        let modified =
            RustAdapter::update_dependency_version(&manifest, "my-dep", &Version::new(3, 0, 0))
                .unwrap();
        assert!(modified);

        let updated = std::fs::read_to_string(&manifest).unwrap();
        assert!(updated.contains("version = \"3.0.0\""));
    }

    #[test]
    fn test_update_dependency_in_workspace_deps() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        let content = "\
[package]\n\
name = \"test\"\n\
version = \"1.0.0\"\n\
\n\
[workspace.dependencies]\n\
my-dep = { version = \"1.0.0\" }\n";
        std::fs::write(&manifest, content).unwrap();

        let modified =
            RustAdapter::update_dependency_version(&manifest, "my-dep", &Version::new(4, 0, 0))
                .unwrap();
        assert!(modified);

        let updated = std::fs::read_to_string(&manifest).unwrap();
        assert!(updated.contains("version = \"4.0.0\""));
    }

    #[test]
    fn publish_dry_run_returns_success() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"test\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let pkg = Package {
            name: "test".to_string(),
            version: Version::new(1, 0, 0),
            path: dir.path().to_path_buf(),
            manifest_path: manifest,
            dependencies: vec![],
        };

        let result = RustAdapter::publish(&pkg, true, None).unwrap();
        assert_eq!(result, PublishResult::Success);
    }

    #[test]
    fn publish_skipped_without_token() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"test\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let pkg = Package {
            name: "test".to_string(),
            version: Version::new(1, 0, 0),
            path: dir.path().to_path_buf(),
            manifest_path: manifest,
            dependencies: vec![],
        };

        // SAFETY: test-only, no concurrent access to this env var
        unsafe {
            std::env::remove_var("CARGO_REGISTRY_TOKEN");
        }
        let result = RustAdapter::publish(&pkg, false, None).unwrap();
        assert_eq!(result, PublishResult::Skipped(SkipReason::NoToken));
    }

    #[test]
    fn publish_skipped_when_publish_false() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"test\"\nversion = \"1.0.0\"\npublish = false\n",
        )
        .unwrap();

        let pkg = Package {
            name: "test".to_string(),
            version: Version::new(1, 0, 0),
            path: dir.path().to_path_buf(),
            manifest_path: manifest,
            dependencies: vec![],
        };

        let result = RustAdapter::publish(&pkg, false, None).unwrap();
        assert_eq!(result, PublishResult::Skipped(SkipReason::NotPublishable));
    }

    #[test]
    fn test_read_version_workspace_inherited() {
        let dir = TempDir::new().unwrap();

        // Root Cargo.toml with workspace.package.version
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/foo\"]\n\n[workspace.package]\nversion = \"2.5.0\"\n",
        )
        .unwrap();

        // Crate with version.workspace = true
        let crate_dir = dir.path().join("crates").join("foo");
        std::fs::create_dir_all(&crate_dir).unwrap();
        let manifest = crate_dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"foo\"\nversion.workspace = true\n",
        )
        .unwrap();

        let version = RustAdapter::read_version(&manifest).unwrap();
        assert_eq!(version, Version::new(2, 5, 0));
    }

    #[test]
    fn test_write_version_workspace_inherited() {
        let dir = TempDir::new().unwrap();

        // Root Cargo.toml with workspace.package.version
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/foo\"]\n\n[workspace.package]\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        // Crate with version.workspace = true
        let crate_dir = dir.path().join("crates").join("foo");
        std::fs::create_dir_all(&crate_dir).unwrap();
        let manifest = crate_dir.join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"foo\"\nversion.workspace = true\n",
        )
        .unwrap();

        RustAdapter::write_version(&manifest, &Version::new(3, 0, 0)).unwrap();

        // Root should be updated
        let root_content = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(root_content.contains("version = \"3.0.0\""));

        // Crate should be untouched
        let crate_content = std::fs::read_to_string(&manifest).unwrap();
        assert!(crate_content.contains("version.workspace = true"));
    }

    #[test]
    fn publish_failed_error_includes_context() {
        let err = crate::error::Error::PublishFailed(
            "cargo publish failed (exit code 101):\nstdout: \nstderr: error: failed to publish"
                .to_string(),
        );
        let msg = err.to_string();
        assert!(msg.contains("publish failed"));
        assert!(msg.contains("exit code 101"));
        assert!(msg.contains("failed to publish"));
    }
}
