use crate::ecosystems::{Ecosystem, EcosystemAdapter, Package, PublishResult};
use crate::error::{Error, Result};
use regex::Regex;
use semver::Version;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

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

const VERSION_COMMENT_PREFIX: &str = "// changelogs:version ";

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
        fs::write(manifest_path, upsert_version_comment(&content, version))?;
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

        Ok(git_tag_exists(version))
    }

    fn publish(_pkg: &Package, _dry_run: bool, _registry: Option<&str>) -> Result<PublishResult> {
        // SwiftPM packages are published by pushing git tags. The shared publish
        // flow creates those tags after this adapter reports success.
        Ok(PublishResult::Success)
    }

    fn tag_name(pkg: &Package) -> String {
        format!("v{}", pkg.version)
    }
}

impl SwiftAdapter {
    pub fn update_all_dependency_versions(
        packages: &[Package],
        _root: &Path,
        updates: &HashMap<String, Version>,
    ) -> Result<()> {
        for pkg in packages {
            for (dep, new_version) in updates {
                Self::update_dependency_version(&pkg.manifest_path, dep, new_version)?;
            }
        }
        Ok(())
    }
}

fn parse_package_name(content: &str) -> Option<String> {
    let re = Regex::new(r#"Package\s*\(\s*name\s*:\s*"([^"]+)""#).ok()?;
    re.captures(content)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
}

fn read_version_from_content(content: &str) -> Option<Version> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(VERSION_COMMENT_PREFIX) {
            if let Ok(version) = rest.trim().parse::<Version>() {
                return Some(version);
            }
        }
    }
    None
}

fn upsert_version_comment(content: &str, version: &Version) -> String {
    let new_line = format!("{}{}", VERSION_COMMENT_PREFIX, version);
    let mut found = false;
    let mut out = Vec::with_capacity(content.lines().count() + 1);

    for line in content.lines() {
        if line.trim().starts_with(VERSION_COMMENT_PREFIX) {
            out.push(new_line.clone());
            found = true;
        } else {
            out.push(line.to_string());
        }
    }

    if !found {
        let insert_idx = out
            .iter()
            .position(|line| line.trim_start().starts_with("// swift-tools-version:"))
            .map(|idx| idx + 1)
            .unwrap_or(0);
        out.insert(insert_idx, new_line);
    }

    let mut joined = out.join("\n");
    if content.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

fn parse_dependencies(content: &str) -> Vec<String> {
    let re = Regex::new(r#"\.package\s*\(\s*(?:name\s*:\s*"[^"]+"\s*,\s*)?url\s*:\s*"([^"]+)""#)
        .expect("valid dependency regex");
    re.captures_iter(content)
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
    let package_re = Regex::new(r#"\.package\s*\([^)]*\)"#).expect("valid package regex");
    let version_re =
        Regex::new(r#"(,\s*(?:from|exact)\s*:\s*)"[^"]+""#).expect("valid version regex");

    let updated = package_re.replace_all(content, |captures: &regex::Captures<'_>| {
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
    let url_re = Regex::new(r#"url\s*:\s*"([^"]+)""#).expect("valid url regex");
    url_re
        .captures(package_call)
        .and_then(|captures| captures.get(1))
        .map(|url| dependency_name_from_url(url.as_str()) == dep_name || url.as_str() == dep_name)
        .unwrap_or(false)
}

fn latest_git_tag_version(repo_dir: &Path) -> Option<Version> {
    let output = Command::new("git")
        .args(["tag", "--list", "v*", "--sort=-version:refname"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let candidate = line.trim().trim_start_matches('v');
        if let Ok(version) = candidate.parse::<Version>() {
            return Some(version);
        }
    }
    None
}

fn git_tag_exists(version: &Version) -> bool {
    let tag = format!("v{}", version);
    let output = Command::new("git")
        .args([
            "rev-parse",
            "--quiet",
            "--verify",
            &format!("refs/tags/{tag}"),
        ])
        .output();

    output
        .map(|output| output.status.success())
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
        let updated = upsert_version_comment(content, &Version::new(0, 1, 0));
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
