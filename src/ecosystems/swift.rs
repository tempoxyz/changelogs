use super::version_comment::{self, latest_git_tag_version, read_version_from_content};
use crate::ecosystems::{Ecosystem, EcosystemAdapter, Package, PublishResult};
use crate::error::{Error, Result};
use regex::Regex;
use semver::Version;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

/// Adapter for Swift Package Manager packages.
///
/// SwiftPM versions are distributed through git tags, not stored in
/// `Package.swift`. Like the Go adapter, we persist the currently planned
/// version in a manifest comment so version bumps survive between CI steps:
///
/// ```swift
/// // changelogs:version 1.2.3
/// // swift-tools-version: 5.10
/// ```
///
/// The adapter currently supports a single root `Package.swift`.
pub struct SwiftAdapter;

impl EcosystemAdapter for SwiftAdapter {
    fn ecosystem() -> Ecosystem {
        Ecosystem::Swift
    }

    fn discover(root: &Path) -> Result<Vec<Package>> {
        let manifest_path = root.join("Package.swift");
        if !manifest_path.exists() {
            return Err(Error::SwiftPackageNotFound(format!(
                "No Package.swift found at {}",
                root.display()
            )));
        }

        let content = fs::read_to_string(&manifest_path)?;
        let name = parse_package_name(&content).ok_or_else(|| {
            Error::SwiftPackageNotFound(format!(
                "Package.swift at {} has no Package(name:) value",
                manifest_path.display()
            ))
        })?;
        let version = read_version_from_content(&content)
            .or_else(|| latest_git_tag_version(root))
            .unwrap_or_else(|| Version::new(0, 0, 0));
        let dependencies = parse_dependencies(&content);

        Ok(vec![Package {
            name,
            version,
            path: root.to_path_buf(),
            manifest_path,
            dependencies,
            publishable: true,
        }])
    }

    fn read_version(manifest_path: &Path) -> Result<Version> {
        let content = fs::read_to_string(manifest_path)?;
        if let Some(version) = read_version_from_content(&content) {
            return Ok(version);
        }

        let root = manifest_path
            .parent()
            .ok_or_else(|| Error::VersionNotFound(manifest_path.display().to_string()))?;
        Ok(latest_git_tag_version(root).unwrap_or_else(|| Version::new(0, 0, 0)))
    }

    fn write_version(manifest_path: &Path, version: &Version) -> Result<()> {
        let content = fs::read_to_string(manifest_path)?;
        fs::write(manifest_path, upsert_version_comment(&content, version)?)?;
        Ok(())
    }

    fn update_dependency_version(
        manifest_path: &Path,
        dep_name: &str,
        new_version: &Version,
    ) -> Result<bool> {
        let content = fs::read_to_string(manifest_path)?;
        let (updated, modified) = rewrite_dependency_versions(&content, dep_name, new_version);
        if modified {
            fs::write(manifest_path, updated)?;
        }
        Ok(modified)
    }

    fn is_published(_name: &str, version: &Version) -> Result<bool> {
        // `0.0.0` is the implicit baseline when no changelogs version comment or
        // git tag exists. Treat it as already published so `publish` does not
        // create a meaningless bootstrap tag.
        if *version == Version::new(0, 0, 0) {
            return Ok(true);
        }

        Ok(super::git_tag_exists(
            Path::new("."),
            &super::root_tag_name(version),
        ))
    }

    fn publish(_pkg: &Package, _dry_run: bool, _registry: Option<&str>) -> Result<PublishResult> {
        // SwiftPM packages are published by pushing git tags. The shared publish
        // flow creates those tags after this adapter reports success.
        Ok(PublishResult::Success)
    }

    fn tag_name(pkg: &Package) -> String {
        super::root_tag_name(&pkg.version)
    }
}

static PACKAGE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"Package\s*\(\s*name\s*:\s*"([^"]+)""#).expect("valid package name regex")
});
static DEPENDENCY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\.package\s*\(\s*(?:name\s*:\s*"[^"]+"\s*,\s*)?url\s*:\s*"([^"]+)""#)
        .expect("valid dependency regex")
});
static PACKAGE_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.package\s*\([^)]*\)"#).expect("valid package regex"));
static DEPENDENCY_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(,\s*(?:from|exact)\s*:\s*)"[^"]+""#).expect("valid version regex")
});
static DEPENDENCY_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"url\s*:\s*"([^"]+)""#).expect("valid url regex"));

fn parse_package_name(content: &str) -> Option<String> {
    PACKAGE_NAME
        .captures(content)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
}

fn upsert_version_comment(content: &str, version: &Version) -> Result<String> {
    // Insert just after the `// swift-tools-version:` line, or first.
    version_comment::upsert_version_comment(content, version, |lines| {
        Ok(lines
            .iter()
            .position(|line| line.trim_start().starts_with("// swift-tools-version:"))
            .map(|idx| idx + 1)
            .unwrap_or(0))
    })
}

fn parse_dependencies(content: &str) -> Vec<String> {
    DEPENDENCY
        .captures_iter(content)
        .filter_map(|captures| captures.get(1))
        .map(|m| dependency_name_from_url(m.as_str()))
        .collect()
}

fn dependency_name_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
}

fn rewrite_dependency_versions(
    content: &str,
    dep_name: &str,
    new_version: &Version,
) -> (String, bool) {
    let mut modified = false;
    let target = format!(r#"$1"{}""#, new_version);
    let version_re = &*DEPENDENCY_VERSION;

    let updated = PACKAGE_CALL.replace_all(content, |captures: &regex::Captures<'_>| {
        let package = captures.get(0).expect("whole match").as_str();
        if !dependency_matches(package, dep_name) || !version_re.is_match(package) {
            return package.to_string();
        }
        modified = true;
        version_re.replace(package, target.as_str()).to_string()
    });

    (updated.into_owned(), modified)
}

fn dependency_matches(package_call: &str, dep_name: &str) -> bool {
    DEPENDENCY_URL
        .captures(package_call)
        .and_then(|captures| captures.get(1))
        .map(|url| dependency_name_from_url(url.as_str()) == dep_name || url.as_str() == dep_name)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_package_name() {
        let content = r#"let package = Package(
            name: "TempoKit",
            products: []
        )"#;
        assert_eq!(parse_package_name(content), Some("TempoKit".to_string()));
    }

    #[test]
    fn reads_version_comment() {
        let content = "// changelogs:version 1.2.3\n// swift-tools-version: 5.10\n";
        assert_eq!(
            read_version_from_content(content),
            Some(Version::new(1, 2, 3))
        );
    }

    #[test]
    fn upsert_inserts_after_tools_version() {
        let content = "// swift-tools-version: 5.10\nimport PackageDescription\n";
        let updated = upsert_version_comment(content, &Version::new(0, 1, 0)).unwrap();
        assert!(updated.starts_with("// swift-tools-version: 5.10\n// changelogs:version 0.1.0\n"));
    }

    #[test]
    fn parses_dependencies_by_package_url_basename() {
        let content = r#".package(url: "https://github.com/apple/swift-log.git", from: "1.5.0"),
            .package(url: "https://github.com/apple/swift-collections", exact: "1.1.0"),"#;
        assert_eq!(
            parse_dependencies(content),
            vec!["swift-log", "swift-collections"]
        );
    }

    #[test]
    fn rewrites_matching_dependency_from_version() {
        let content = r#".package(url: "https://github.com/apple/swift-log.git", from: "1.5.0"),
            .package(url: "https://github.com/apple/swift-collections.git", from: "1.1.0"),"#;
        let (updated, modified) =
            rewrite_dependency_versions(content, "swift-log", &Version::new(1, 6, 0));
        assert!(modified);
        assert!(updated.contains(r#"swift-log.git", from: "1.6.0""#));
        assert!(updated.contains(r#"swift-collections.git", from: "1.1.0""#));
    }

    #[test]
    fn is_published_skips_bootstrap_v0_0_0() {
        assert!(SwiftAdapter::is_published("TempoKit", &Version::new(0, 0, 0)).unwrap());
    }
}
