use crate::config::{ChangelogFormat, Config};
use crate::ecosystems::{self, Ecosystem, Package, PublishResult};
use crate::error::{Error, Result};
use semver::Version;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub packages: Vec<Package>,
    pub ecosystem: Ecosystem,
    /// The workspace's `.changelog/config.toml`, loaded once at discovery.
    pub config: Config,
}

pub type WorkspacePackage = Package;

impl Workspace {
    pub fn discover() -> Result<Self> {
        Self::discover_with_ecosystem(None)
    }

    pub fn discover_with_ecosystem(ecosystem: Option<Ecosystem>) -> Result<Self> {
        let cwd = std::env::current_dir()?;
        Self::discover_from(&cwd, ecosystem)
    }

    fn discover_from(cwd: &Path, ecosystem: Option<Ecosystem>) -> Result<Self> {
        let ecosystem = ecosystem
            .or_else(|| ecosystems::detect_ecosystem(cwd))
            .ok_or(Error::NotInWorkspace)?;

        let root = Self::find_root(cwd, ecosystem)?;
        let config = Config::load(&root.join(".changelog"))?;
        let packages =
            ecosystems::discover_packages_with_private(ecosystem, &root, &config.private)?;

        if packages.is_empty() {
            return Err(Error::NotInWorkspace);
        }

        Ok(Workspace {
            root,
            packages,
            ecosystem,
            config,
        })
    }

    fn find_root(start: &Path, ecosystem: Ecosystem) -> Result<PathBuf> {
        let manifest_name = match ecosystem {
            Ecosystem::Rust => "Cargo.toml",
            Ecosystem::Python => "pyproject.toml",
            Ecosystem::Go => "go.mod",
            Ecosystem::Swift => "Package.swift",
        };

        let mut current = start.to_path_buf();

        loop {
            let manifest = current.join(manifest_name);
            if manifest.exists() {
                if ecosystem == Ecosystem::Rust {
                    let content = std::fs::read_to_string(&manifest)?;
                    if content.contains("[workspace]") {
                        return Ok(current);
                    }
                }

                let parent = current.parent();
                if parent.is_none() {
                    return Ok(current);
                }

                let parent_manifest = parent.unwrap().join(manifest_name);
                if !parent_manifest.exists() {
                    return Ok(current);
                }

                if ecosystem == Ecosystem::Rust {
                    let content = std::fs::read_to_string(&parent_manifest)?;
                    if content.contains("[workspace]") {
                        current = parent.unwrap().to_path_buf();
                        continue;
                    }
                }

                return Ok(current);
            }

            match current.parent() {
                Some(parent) => current = parent.to_path_buf(),
                None => return Err(Error::NotInWorkspace),
            }
        }
    }

    pub fn load() -> Result<Self> {
        Self::discover()
    }

    pub fn load_with_ecosystem(ecosystem: Option<Ecosystem>) -> Result<Self> {
        Self::discover_with_ecosystem(ecosystem)
    }

    pub fn changelog_dir(&self) -> PathBuf {
        self.root.join(".changelog")
    }

    pub fn get_publishable_packages(&self) -> Result<Vec<&Package>> {
        let mut publishable = Vec::new();

        for pkg in &self.packages {
            let private = self.is_private_package(pkg);
            if self.release_is_tagged(pkg, private) {
                continue;
            }

            if private || !ecosystems::is_published(self.ecosystem, &pkg.name, &pkg.version)? {
                publishable.push(pkg);
            }
        }

        Ok(publishable)
    }

    pub fn is_initialized(&self) -> bool {
        self.changelog_dir().exists()
    }

    pub fn get_package(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn package_names(&self) -> Vec<&str> {
        self.packages.iter().map(|p| p.name.as_str()).collect()
    }

    pub fn unified_package(&self) -> Option<&Package> {
        self.packages
            .iter()
            .find(|package| package.path == self.root)
            .or_else(|| self.packages.iter().min_by_key(|package| &package.path))
    }

    pub fn update_version(&self, package_name: &str, new_version: &Version) -> Result<()> {
        let package = self
            .get_package(package_name)
            .ok_or_else(|| Error::PackageNotFound(package_name.to_string()))?;

        ecosystems::write_version(self.ecosystem, &package.manifest_path, new_version)
    }

    pub fn update_dependency_versions(&self, updates: &HashMap<String, Version>) -> Result<()> {
        ecosystems::update_dependency_versions(self.ecosystem, &self.packages, &self.root, updates)
    }

    pub fn publish_package(
        &self,
        pkg: &Package,
        dry_run: bool,
        registry: Option<&str>,
    ) -> Result<PublishResult> {
        if self.ecosystem == Ecosystem::Rust && self.config.private.contains(&pkg.name) {
            return Ok(PublishResult::Skipped(
                crate::ecosystems::SkipReason::NotPublishable,
            ));
        }

        ecosystems::publish(self.ecosystem, pkg, dry_run, registry)
    }

    pub fn tag_name(&self, pkg: &Package) -> String {
        ecosystems::tag_name(self.ecosystem, pkg)
    }

    /// The tag `publish` creates for `pkg`'s release: the unified `v{version}`
    /// tag in root format, the package's own tag otherwise.
    pub fn release_tag(&self, pkg: &Package) -> String {
        match (self.config.changelog.format, self.unified_package()) {
            (ChangelogFormat::Root, Some(unified)) => ecosystems::root_tag_name(&unified.version),
            _ => self.tag_name(pkg),
        }
    }

    pub fn tag_exists(&self, tag: &str) -> bool {
        ecosystems::git_tag_exists(&self.root, tag)
    }

    fn is_private_package(&self, pkg: &Package) -> bool {
        self.ecosystem == Ecosystem::Rust
            && (self.config.private.contains(&pkg.name) || !pkg.publishable)
    }

    /// Whether this package version's release tag already exists, so a rerun
    /// of `publish` treats it as released.
    ///
    /// In root format the unified `v{version}` tag covers private packages and
    /// the unified package itself; other public packages rely on the registry.
    fn release_is_tagged(&self, pkg: &Package, private: bool) -> bool {
        let tag = if self.config.changelog.format == ChangelogFormat::Root {
            let unified = self.unified_package().unwrap_or(pkg);
            if !private && unified.name != pkg.name {
                return false;
            }
            if private && unified.version == Version::new(0, 0, 0) {
                return true;
            }
            ecosystems::root_tag_name(&unified.version)
        } else {
            if private && pkg.version == Version::new(0, 0, 0) {
                return true;
            }
            self.tag_name(pkg)
        };

        self.tag_exists(&tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystems::Package;
    use std::process::Command;
    use tempfile::TempDir;

    fn run_git(root: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .unwrap()
                .success()
        );
    }

    fn make_package(name: &str) -> Package {
        Package {
            name: name.to_string(),
            version: Version::new(1, 0, 0),
            path: PathBuf::from(format!("/fake/{name}")),
            manifest_path: PathBuf::from(format!("/fake/{name}/Cargo.toml")),
            dependencies: vec![],
            publishable: true,
        }
    }

    fn make_workspace(root: PathBuf, packages: Vec<Package>) -> Workspace {
        Workspace {
            root,
            packages,
            ecosystem: Ecosystem::Rust,
            config: Config::default(),
        }
    }

    fn private_tool_config() -> Config {
        Config {
            private: vec!["private-tool".into()],
            ..Default::default()
        }
    }

    fn write_rust_workspace(root: &Path, root_private: bool) {
        let publish = if root_private {
            "publish = false\n"
        } else {
            ""
        };
        std::fs::write(
            root.join("Cargo.toml"),
            format!(
                r#"[package]
name = "product"
version = "1.0.0"
edition = "2021"
{publish}
[workspace]
members = [".", "helper"]
"#
            ),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("lib.rs"), "").unwrap();
        std::fs::create_dir_all(root.join("helper").join("src")).unwrap();
        std::fs::write(
            root.join("helper").join("Cargo.toml"),
            r#"[package]
name = "helper"
version = "9.0.0"
edition = "2021"
publish = false
"#,
        )
        .unwrap();
        std::fs::write(root.join("helper").join("src").join("lib.rs"), "").unwrap();
    }

    #[test]
    fn test_get_package() {
        let ws = make_workspace(
            PathBuf::from("/tmp/proj"),
            vec![make_package("foo"), make_package("bar")],
        );

        let pkg = ws.get_package("foo").unwrap();
        assert_eq!(pkg.name, "foo");

        assert!(ws.get_package("nonexistent").is_none());
    }

    #[test]
    fn test_package_names() {
        let ws = make_workspace(
            PathBuf::from("/tmp/proj"),
            vec![
                make_package("alpha"),
                make_package("beta"),
                make_package("gamma"),
            ],
        );

        let names = ws.package_names();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn test_is_initialized_true() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".changelog")).unwrap();
        let ws = make_workspace(dir.path().to_path_buf(), vec![]);

        assert!(ws.is_initialized());
    }

    #[test]
    fn test_is_initialized_false() {
        let dir = TempDir::new().unwrap();
        let ws = make_workspace(dir.path().to_path_buf(), vec![]);

        assert!(!ws.is_initialized());
    }

    #[test]
    fn test_private_config_skips_registry_lookup() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"private-tool\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let pkg = Package {
            manifest_path: manifest,
            path: dir.path().to_path_buf(),
            ..make_package("private-tool")
        };
        let workspace = Workspace {
            config: private_tool_config(),
            ..make_workspace(dir.path().to_path_buf(), vec![pkg.clone()])
        };

        assert!(workspace.is_private_package(&pkg));
    }

    #[test]
    fn test_private_config_skips_registry_publish() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"private-tool\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let pkg = Package {
            manifest_path: manifest,
            path: dir.path().to_path_buf(),
            ..make_package("private-tool")
        };
        let workspace = Workspace {
            config: private_tool_config(),
            ..make_workspace(dir.path().to_path_buf(), vec![pkg.clone()])
        };

        assert_eq!(
            workspace.publish_package(&pkg, true, None).unwrap(),
            PublishResult::Skipped(crate::ecosystems::SkipReason::NotPublishable)
        );
    }

    #[test]
    fn test_private_tagged_package_is_not_publishable() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"private-tool\"\nversion = \"1.0.0\"\npublish = false\n",
        )
        .unwrap();
        let pkg = Package {
            manifest_path: manifest,
            path: dir.path().to_path_buf(),
            publishable: false,
            ..make_package("private-tool")
        };
        let workspace = Workspace {
            config: Config {
                changelog: crate::config::ChangelogConfig {
                    format: ChangelogFormat::Root,
                },
                ..private_tool_config()
            },
            ..make_workspace(dir.path().to_path_buf(), vec![pkg])
        };
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        run_git(dir.path(), &["add", "."]);
        run_git(dir.path(), &["commit", "-qm", "initial"]);
        run_git(dir.path(), &["tag", "v1.0.0"]);

        assert!(workspace.get_publishable_packages().unwrap().is_empty());
    }

    #[test]
    fn test_private_config_preserves_swift_bootstrap_guard() {
        let dir = TempDir::new().unwrap();
        let pkg = Package {
            manifest_path: dir.path().join("Package.swift"),
            path: dir.path().to_path_buf(),
            version: Version::new(0, 0, 0),
            ..make_package("private-tool")
        };
        let workspace = Workspace {
            ecosystem: Ecosystem::Swift,
            config: private_tool_config(),
            ..make_workspace(dir.path().to_path_buf(), vec![pkg])
        };

        assert!(workspace.get_publishable_packages().unwrap().is_empty());
    }

    #[test]
    fn test_private_rust_package_preserves_bootstrap_guard() {
        let dir = TempDir::new().unwrap();
        let pkg = Package {
            manifest_path: dir.path().join("Cargo.toml"),
            path: dir.path().to_path_buf(),
            version: Version::new(0, 0, 0),
            ..make_package("private-tool")
        };
        let workspace = Workspace {
            config: private_tool_config(),
            ..make_workspace(dir.path().to_path_buf(), vec![pkg])
        };

        assert!(workspace.get_publishable_packages().unwrap().is_empty());
    }

    #[test]
    fn test_publish_false_skips_registry_lookup() {
        let dir = TempDir::new().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"private-tool\"\nversion = \"1.0.0\"\npublish = false\n",
        )
        .unwrap();
        let pkg = Package {
            manifest_path: manifest,
            path: dir.path().to_path_buf(),
            publishable: false,
            ..make_package("private-tool")
        };
        let workspace = make_workspace(dir.path().to_path_buf(), vec![pkg.clone()]);

        assert!(workspace.is_private_package(&pkg));
    }

    #[test]
    fn test_changelog_dir() {
        let ws = make_workspace(PathBuf::from("/tmp/myproject"), vec![]);
        assert_eq!(
            ws.changelog_dir(),
            PathBuf::from("/tmp/myproject/.changelog")
        );
    }

    #[test]
    fn test_discover_from_config_includes_private_in_mixed_workspace() {
        let dir = TempDir::new().unwrap();
        write_rust_workspace(dir.path(), false);
        let changelog_dir = dir.path().join(".changelog");
        std::fs::create_dir(&changelog_dir).unwrap();
        Config {
            private: vec!["helper".into()],
            ..Default::default()
        }
        .save(&changelog_dir)
        .unwrap();

        let workspace = Workspace::discover_from(dir.path(), Some(Ecosystem::Rust)).unwrap();
        let mut names = workspace.package_names();
        names.sort_unstable();

        assert_eq!(names, vec!["helper", "product"]);
    }

    #[test]
    fn test_discover_from_config_narrows_all_private_workspace() {
        let dir = TempDir::new().unwrap();
        write_rust_workspace(dir.path(), true);
        let changelog_dir = dir.path().join(".changelog");
        std::fs::create_dir(&changelog_dir).unwrap();
        Config {
            private: vec!["product".into()],
            ..Default::default()
        }
        .save(&changelog_dir)
        .unwrap();

        let workspace = Workspace::discover_from(dir.path(), Some(Ecosystem::Rust)).unwrap();

        assert_eq!(workspace.package_names(), vec!["product"]);
    }

    #[test]
    fn test_discover_from_config_reports_unknown_private_package() {
        let dir = TempDir::new().unwrap();
        write_rust_workspace(dir.path(), true);
        let changelog_dir = dir.path().join(".changelog");
        std::fs::create_dir(&changelog_dir).unwrap();
        Config {
            private: vec!["missing".into()],
            ..Default::default()
        }
        .save(&changelog_dir)
        .unwrap();

        let error = Workspace::discover_from(dir.path(), Some(Ecosystem::Rust)).unwrap_err();

        assert!(matches!(error, Error::UnknownPrivatePackage(package) if package == "missing"));
    }

    #[test]
    fn test_find_root_rust_workspace() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"foo\"]\n",
        )
        .unwrap();

        let crate_dir = root.join("foo");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let found = Workspace::find_root(&crate_dir, Ecosystem::Rust).unwrap();
        assert_eq!(found, root);
    }

    #[test]
    fn test_find_root_rust_no_workspace() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"solo\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let found = Workspace::find_root(dir.path(), Ecosystem::Rust).unwrap();
        assert_eq!(found, dir.path());
    }

    #[test]
    fn test_find_root_python() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"mypy\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let found = Workspace::find_root(dir.path(), Ecosystem::Python).unwrap();
        assert_eq!(found, dir.path());
    }

    #[test]
    fn test_find_root_not_found() {
        let dir = TempDir::new().unwrap();
        let result = Workspace::find_root(dir.path(), Ecosystem::Rust);
        assert!(result.is_err());
    }
}
