//! Shared by the ecosystems whose versions live in git tags (Go and Swift).
//! They persist the planned version in a manifest comment, so a version bump
//! survives between the `version` and `publish` CI steps.

use crate::error::Result;
use semver::Version;
use std::path::Path;
use std::process::Command;

pub(super) const VERSION_COMMENT_PREFIX: &str = "// changelogs:version ";

pub(super) fn read_version_from_content(content: &str) -> Option<Version> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(VERSION_COMMENT_PREFIX)
            && let Ok(version) = rest.trim().parse::<Version>()
        {
            return Some(version);
        }
    }
    None
}

/// Replaces the version comment in `content`, or, when there is none, inserts
/// it at the line index `insert_at` picks from the manifest's lines.
pub(super) fn upsert_version_comment(
    content: &str,
    version: &Version,
    insert_at: impl FnOnce(&[String]) -> Result<usize>,
) -> Result<String> {
    let new_line = format!("{}{}", VERSION_COMMENT_PREFIX, version);
    let mut found = false;
    let mut out: Vec<String> = Vec::with_capacity(content.lines().count() + 1);

    for line in content.lines() {
        if line.trim().starts_with(VERSION_COMMENT_PREFIX) {
            out.push(new_line.clone());
            found = true;
        } else {
            out.push(line.to_string());
        }
    }

    if !found {
        let idx = insert_at(&out)?;
        out.insert(idx, new_line);
    }

    let mut joined = out.join("\n");
    if content.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

/// The highest version among the repository's `v*` tags.
pub(super) fn latest_git_tag_version(repo_dir: &Path) -> Option<Version> {
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
