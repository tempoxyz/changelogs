mod python;
mod rust;

pub use python::PythonAdapter;
pub use rust::RustAdapter;

use crate::error::Result;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Ecosystem {
    #[default]
    Rust,
    Python,
}

impl Ecosystem {
    const RUST_ALIASES: &[&str] = &["rust", "cargo"];
    const PYTHON_ALIASES: &[&str] = &["python", "pypi"];

    pub fn from_alias(s: &str) -> Option<Self> {
        let lower = s.to_lowercase();
        if Self::RUST_ALIASES.contains(&lower.as_str()) {
            Some(Ecosystem::Rust)
        } else if Self::PYTHON_ALIASES.contains(&lower.as_str()) {
            Some(Ecosystem::Python)
        } else {
            None
        }
    }
}

/// Result of attempting to publish a package
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishResult {
    /// Package was successfully published
    Success,
    /// Publishing was skipped
    Skipped(SkipReason),
    /// Publishing failed
    Failed,
}

/// Reason a package publish was skipped
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// No registry token configured
    NoToken,
    /// Package has `publish = false`
    NotPublishable,
}

impl std::fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ecosystem::Rust => write!(f, "rust"),
            Ecosystem::Python => write!(f, "python"),
        }
    }
}

impl std::str::FromStr for Ecosystem {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::from_alias(s).ok_or_else(|| crate::error::Error::InvalidEcosystem(s.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub version: Version,
    pub path: PathBuf,
    pub manifest_path: PathBuf,
    pub dependencies: Vec<String>,
}

/// Trait defining ecosystem-specific operations for package management.
///
/// Note: Methods are associated functions (not instance methods) because adapters
/// are stateless. Use the free functions in this module for dispatch by `Ecosystem` enum.
pub trait EcosystemAdapter {
    /// Returns the ecosystem identifier.
    fn ecosystem() -> Ecosystem
    where
        Self: Sized;

    /// Discovers packages in the given root directory.
    fn discover(root: &Path, skip_dep_resolution: bool) -> Result<Vec<Package>>
    where
        Self: Sized;

    /// Reads the version from a manifest file.
    fn read_version(manifest_path: &Path) -> Result<Version>
    where
        Self: Sized;

    /// Writes a new version to a manifest file.
    fn write_version(manifest_path: &Path, version: &Version) -> Result<()>
    where
        Self: Sized;

    /// Updates a dependency version in a manifest file. Returns true if modified.
    fn update_dependency_version(
        manifest_path: &Path,
        dep_name: &str,
        new_version: &Version,
    ) -> Result<bool>
    where
        Self: Sized;

    /// Checks if a package version is already published to the registry.
    fn is_published(name: &str, version: &Version) -> Result<bool>
    where
        Self: Sized;

    /// Publishes a package to the registry.
    fn publish(pkg: &Package, dry_run: bool, registry: Option<&str>) -> Result<PublishResult>
    where
        Self: Sized;

    /// Returns the git tag name for a package release.
    fn tag_name(pkg: &Package) -> String
    where
        Self: Sized,
    {
        format!("{}@{}", pkg.name, pkg.version)
    }
}

pub fn detect_ecosystem(start: &Path) -> Option<Ecosystem> {
    let mut current = start.to_path_buf();

    loop {
        if current.join("Cargo.toml").exists() {
            return Some(Ecosystem::Rust);
        }
        if current.join("pyproject.toml").exists() {
            return Some(Ecosystem::Python);
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return None,
        }
    }
}

pub fn discover_packages(
    ecosystem: Ecosystem,
    root: &Path,
    skip_dep_resolution: bool,
) -> Result<Vec<Package>> {
    match ecosystem {
        Ecosystem::Rust => RustAdapter::discover(root, skip_dep_resolution),
        Ecosystem::Python => PythonAdapter::discover(root, skip_dep_resolution),
    }
}

pub fn read_version(ecosystem: Ecosystem, manifest_path: &Path) -> Result<Version> {
    match ecosystem {
        Ecosystem::Rust => RustAdapter::read_version(manifest_path),
        Ecosystem::Python => PythonAdapter::read_version(manifest_path),
    }
}

pub fn write_version(ecosystem: Ecosystem, manifest_path: &Path, version: &Version) -> Result<()> {
    match ecosystem {
        Ecosystem::Rust => RustAdapter::write_version(manifest_path, version),
        Ecosystem::Python => PythonAdapter::write_version(manifest_path, version),
    }
}

pub fn update_dependency_versions(
    ecosystem: Ecosystem,
    packages: &[Package],
    root: &Path,
    updates: &HashMap<String, Version>,
) -> Result<()> {
    match ecosystem {
        Ecosystem::Rust => RustAdapter::update_all_dependency_versions(packages, root, updates),
        Ecosystem::Python => PythonAdapter::update_all_dependency_versions(packages, root, updates),
    }
}

pub fn is_published(ecosystem: Ecosystem, name: &str, version: &Version) -> Result<bool> {
    match ecosystem {
        Ecosystem::Rust => RustAdapter::is_published(name, version),
        Ecosystem::Python => PythonAdapter::is_published(name, version),
    }
}

pub fn publish(
    ecosystem: Ecosystem,
    pkg: &Package,
    dry_run: bool,
    registry: Option<&str>,
) -> Result<PublishResult> {
    match ecosystem {
        Ecosystem::Rust => RustAdapter::publish(pkg, dry_run, registry),
        Ecosystem::Python => PythonAdapter::publish(pkg, dry_run, registry),
    }
}

pub fn tag_name(ecosystem: Ecosystem, pkg: &Package) -> String {
    match ecosystem {
        Ecosystem::Rust => RustAdapter::tag_name(pkg),
        Ecosystem::Python => PythonAdapter::tag_name(pkg),
    }
}
