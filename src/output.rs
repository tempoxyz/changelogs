//! Machine-readable output of `--json`. These types are the JSON schema
//! that the GitHub Action and other scripts rely on: add fields, but do not
//! rename or remove them.

use crate::config::ChangelogFormat;
use serde::Serialize;

/// `changelogs version --json`.
#[derive(Debug, Clone, Serialize)]
pub struct VersionOutput {
    /// `root` or `per-crate`.
    pub format: ChangelogFormat,
    pub releases: Vec<VersionRelease>,
    /// The unified `v{version}` release in root format; null otherwise.
    pub unified_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionRelease {
    pub name: String,
    pub old_version: String,
    pub new_version: String,
    pub bump: crate::BumpType,
    /// Whether this release promotes a prerelease to a stable version.
    pub is_promotion: bool,
    /// The CHANGELOG.md this release is recorded in, relative to the repository root.
    pub changelog_path: String,
    /// The markdown section, with its heading, that was (or with --dry-run
    /// would be) added to `changelog_path`. Empty when none is written.
    pub notes: String,
}

/// `changelogs publish --json`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PublishOutput {
    pub published: Vec<PublishedPackage>,
    pub skipped: Vec<SkippedPackage>,
    pub failed: Vec<FailedPackage>,
    /// Git tags this run created, to push.
    pub tags_created: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublishedPackage {
    pub name: String,
    pub version: String,
    pub tag: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedPackage {
    pub name: String,
    pub version: String,
    pub tag: String,
    /// `no_token` or `not_publishable`.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedPackage {
    pub name: String,
    pub version: String,
    pub error: String,
}

/// Prints `value` as pretty JSON on stdout.
pub fn print_json<T: Serialize>(value: &T) -> serde_json::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
