use crate::ecosystems::{self, Ecosystem, Package, PublishResult};
use crate::error::{Error, Result};
use semver::Version;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub changelog_dir: PathBuf,
    pub packages: Vec<Package>,
    pub ecosystem: Ecosystem,
}

pub type WorkspacePackage = Package;

impl Workspace {
    pub fn discover() -> Result<Self> {
        Self::discover_with_ecosystem(None)
    }

    pub fn discover_with_ecosystem(ecosystem: Option<Ecosystem>) -> Result<Self> {
        let cwd = std::env::current_dir()?;

        let ecosystem = ecosystem
            .or_else(|| ecosystems::detect_ecosystem(&cwd))
            .ok_or(Error::NotInWorkspace)?;

        let root = Self::find_root(&cwd, ecosystem)?;
        let packages = ecosystems::discover_packages(ecosystem, &root)?;

        if packages.is_empty() {
            return Err(Error::NotInWorkspace);
        }

        let changelog_dir = root.join(".changelog");

        Ok(Workspace {
            root,
            changelog_dir,
            packages,
            ecosystem,
        })
    }

    fn find_root(start: &Path, ecosystem: Ecosystem) -> Result<PathBuf> {
        let manifest_name = match ecosystem {
            Ecosystem::Rust => "Cargo.toml",
            Ecosystem::Python => "pyproject.toml",
            Ecosystem::Go => "go.mod",
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
            let is_published = ecosystems::is_published(self.ecosystem, &pkg.name, &pkg.version)?;

            if !is_published {
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
        ecosystems::publish(self.ecosystem, pkg, dry_run, registry)
    }

    pub fn tag_name(&self, pkg: &Package) -> String {
        ecosystems::tag_name(self.ecosystem, pkg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystems::Package;
    use tempfile::TempDir;

    fn make_package(name: &str) -> Package {
        Package {
            name: name.to_string(),
            version: Version::new(1, 0, 0),
            path: PathBuf::from(format!("/fake/{name}")),
            manifest_path: PathBuf::from(format!("/fake/{name}/Cargo.toml")),
            dependencies: vec![],
        }
    }

    fn make_workspace(root: PathBuf, packages: Vec<Package>) -> Workspace {
        let changelog_dir = root.join(".changelog");
        Workspace {
            root,
            changelog_dir,
            packages,
            ecosystem: Ecosystem::Rust,
        }
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
    fn test_changelog_dir() {
        let ws = make_workspace(PathBuf::from("/tmp/myproject"), vec![]);
        assert_eq!(
            ws.changelog_dir(),
            PathBuf::from("/tmp/myproject/.changelog")
        );
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
