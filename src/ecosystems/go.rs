use crate::ecosystems::{Ecosystem, EcosystemAdapter, Package, PublishResult};
use crate::error::{Error, Result};
use semver::Version;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Adapter for Go modules.
///
/// Go is unusual: versions live in **git tags**, not in the manifest. To stay
/// compatible with the existing `version → publish` flow (which needs to remember
/// the bumped version between two CI runs) we persist the version inline as a
/// comment in `go.mod`:
///
/// ```text
/// // changelogs:version 1.2.3
/// module github.com/owner/repo
/// ```
///
/// Currently only **single-module** repositories are supported. Multi-module
/// monorepos (each with their own `go.mod` and tag prefix) are out of scope.
pub struct GoAdapter;

const VERSION_COMMENT_PREFIX: &str = "// changelogs:version ";

impl EcosystemAdapter for GoAdapter {
    fn ecosystem() -> Ecosystem {
        Ecosystem::Go
    }

    fn discover(root: &Path) -> Result<Vec<Package>> {
        let go_mod = root.join("go.mod");
        if !go_mod.exists() {
            return Err(Error::GoModuleNotFound(format!(
                "No go.mod found at {}",
                root.display()
            )));
        }

        let content = fs::read_to_string(&go_mod)?;
        let module_path = parse_module_path(&content).ok_or_else(|| {
            Error::GoModuleNotFound(format!(
                "go.mod at {} has no `module` directive",
                go_mod.display()
            ))
        })?;

        // Use the full module path as the package name. In Go the import path *is*
        // the canonical identifier, and we need it intact so `is_published` can
        // query the module proxy. Users reference packages by this path in
        // changelog frontmatter (e.g. `github.com/owner/repo: minor`).
        let name = module_path;

        // Read version from comment first; fall back to latest matching git tag; else 0.0.0.
        let version = read_version_from_content(&content)
            .or_else(|| latest_git_tag_version(root))
            .unwrap_or_else(|| Version::new(0, 0, 0));

        let dependencies = parse_dependencies(&content);

        Ok(vec![Package {
            name,
            version,
            path: root.to_path_buf(),
            manifest_path: go_mod,
            dependencies,
        }])
    }

    fn read_version(manifest_path: &Path) -> Result<Version> {
        let content = fs::read_to_string(manifest_path)?;
        if let Some(v) = read_version_from_content(&content) {
            return Ok(v);
        }
        // Fall back to walking the git tree from the manifest's parent.
        let root = manifest_path
            .parent()
            .ok_or_else(|| Error::VersionNotFound(manifest_path.display().to_string()))?;
        if let Some(v) = latest_git_tag_version(root) {
            return Ok(v);
        }
        Ok(Version::new(0, 0, 0))
    }

    fn write_version(manifest_path: &Path, version: &Version) -> Result<()> {
        let content = fs::read_to_string(manifest_path)?;
        let updated = upsert_version_comment(&content, version)?;
        fs::write(manifest_path, updated)?;
        Ok(())
    }

    fn update_dependency_version(
        manifest_path: &Path,
        dep_name: &str,
        new_version: &Version,
    ) -> Result<bool> {
        let content = fs::read_to_string(manifest_path)?;
        let (updated, modified) = rewrite_require(&content, dep_name, new_version);
        if modified {
            fs::write(manifest_path, updated)?;
        }
        Ok(modified)
    }

    fn is_published(name: &str, version: &Version) -> Result<bool> {
        // `name` is the full module path (set in `discover`). Query the Go module
        // proxy directly so already-released versions are skipped on republish.
        //
        // Special case: `v0.0.0` is the implicit baseline returned by `discover`
        // when there is no `// changelogs:version` comment in `go.mod` and no
        // existing `vX.Y.Z` git tag — i.e. this module has never had a real
        // release. Treat it as "already published" so the publisher does NOT
        // bootstrap-tag `v0.0.0` on every push to the release branch. A real
        // first release happens once a changelog entry bumps the version
        // (e.g. to `v0.1.0`) and `is_published` queries the proxy for that.
        if *version == Version::new(0, 0, 0) {
            return Ok(true);
        }
        check_proxy_published(name, version)
    }

    fn publish(_pkg: &Package, _dry_run: bool, _registry: Option<&str>) -> Result<PublishResult> {
        // Go modules are published by pushing a git tag — there is no registry upload.
        // The orchestrator (cli/publish.rs → create_git_tags) handles tagging, so we
        // simply report success here so the tagging step runs.
        Ok(PublishResult::Success)
    }

    fn tag_name(pkg: &Package) -> String {
        // Root module: just `vX.Y.Z`. Sub-module support would prepend the module's
        // path within the repo, but we don't expose that here yet.
        format!("v{}", pkg.version)
    }
}

impl GoAdapter {
    /// Bulk-update of dependency versions across all known packages. Mirrors the
    /// Rust/Python signatures so that `ecosystems::update_dependency_versions`
    /// can dispatch uniformly.
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

// -- helpers --------------------------------------------------------------

fn parse_module_path(go_mod: &str) -> Option<String> {
    for line in go_mod.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            // Strip an inline comment if present, then quotes and whitespace.
            let path = rest.split("//").next().unwrap_or("").trim();
            let path = path.trim_matches('"');
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

fn read_version_from_content(content: &str) -> Option<Version> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(VERSION_COMMENT_PREFIX) {
            if let Ok(v) = rest.trim().parse::<Version>() {
                return Some(v);
            }
        }
    }
    None
}

fn upsert_version_comment(content: &str, version: &Version) -> Result<String> {
    let new_line = format!("{}{}", VERSION_COMMENT_PREFIX, version);

    // Replace existing comment if present.
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
        // Insert just before the `module` line. If we somehow lack one, prepend.
        let module_idx = out
            .iter()
            .position(|l| l.trim_start().starts_with("module "))
            .ok_or_else(|| {
                Error::GoModuleNotFound("go.mod has no `module` directive".to_string())
            })?;
        out.insert(module_idx, new_line);
    }

    let mut joined = out.join("\n");
    if content.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

fn parse_dependencies(go_mod: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_block = false;

    for raw in go_mod.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if in_block {
            if line == ")" {
                in_block = false;
                continue;
            }
            if let Some(name) = first_token(line) {
                deps.push(name.to_string());
            }
            continue;
        }
        if line == "require (" {
            in_block = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            // Single-line form: `require github.com/foo/bar v1.2.3`
            if let Some(name) = first_token(rest) {
                deps.push(name.to_string());
            }
        }
    }

    deps
}

fn first_token(s: &str) -> Option<&str> {
    s.split_whitespace().next()
}

fn rewrite_require(content: &str, dep_name: &str, new_version: &Version) -> (String, bool) {
    let mut modified = false;
    let mut in_block = false;
    let mut out: Vec<String> = Vec::with_capacity(content.lines().count());
    let target_version = format!("v{}", new_version);

    for raw in content.lines() {
        let trimmed = raw.trim_start();

        if in_block {
            if trimmed.starts_with(')') {
                in_block = false;
                out.push(raw.to_string());
                continue;
            }
            if let Some(rewritten) = rewrite_require_line(raw, dep_name, &target_version) {
                modified = true;
                out.push(rewritten);
            } else {
                out.push(raw.to_string());
            }
            continue;
        }

        if trimmed == "require (" {
            in_block = true;
            out.push(raw.to_string());
            continue;
        }

        // Single-line form.
        if let Some(rest) = trimmed.strip_prefix("require ") {
            let indent_len = raw.len() - trimmed.len();
            let indent = &raw[..indent_len];
            if let Some(rewritten) = rewrite_require_line(rest, dep_name, &target_version) {
                modified = true;
                out.push(format!("{indent}require {rewritten}"));
                continue;
            }
        }

        out.push(raw.to_string());
    }

    let mut joined = out.join("\n");
    if content.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    (joined, modified)
}

/// Try to rewrite a single line of the form `<indent><dep> <version>[ // comment]`.
/// Returns the new line on a successful match, preserving indent and trailing comment.
fn rewrite_require_line(line: &str, dep_name: &str, new_version: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];

    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let name = parts.next()?;
    if name != dep_name {
        return None;
    }
    let rest = parts.next()?.trim_start();

    // rest looks like: "v1.2.3" optionally followed by " // indirect" etc.
    let (_old_version, suffix) = match rest.find("//") {
        Some(idx) => (rest[..idx].trim_end(), &rest[idx..]),
        None => (rest.trim_end(), ""),
    };

    let mut rebuilt = format!("{indent}{name} {new_version}");
    if !suffix.is_empty() {
        rebuilt.push(' ');
        rebuilt.push_str(suffix);
    }
    Some(rebuilt)
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
        if let Ok(v) = candidate.parse::<Version>() {
            return Some(v);
        }
    }
    None
}

fn check_proxy_published(module_path: &str, version: &Version) -> Result<bool> {
    let escaped = escape_module_path(module_path);
    let url = format!("https://proxy.golang.org/{}/@v/v{}.info", escaped, version);
    match ureq::get(&url).call() {
        Ok(_) => Ok(true),
        Err(ureq::Error::StatusCode(404)) => Ok(false),
        Err(ureq::Error::StatusCode(410)) => Ok(false),
        Err(e) => Err(Error::GoProxyCheckFailed(e.to_string())),
    }
}

/// Module proxy escapes uppercase letters as `!<lower>` (case-encoding).
fn escape_module_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_go_mod(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("go.mod");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parses_module_path_simple() {
        let content = "module github.com/foo/bar\n\ngo 1.22\n";
        assert_eq!(
            parse_module_path(content),
            Some("github.com/foo/bar".to_string())
        );
    }

    #[test]
    fn parses_module_path_with_inline_comment() {
        let content = "module github.com/foo/bar // a comment\n";
        assert_eq!(
            parse_module_path(content),
            Some("github.com/foo/bar".to_string())
        );
    }

    #[test]
    fn parses_module_path_quoted() {
        let content = "module \"github.com/foo/bar\"\n";
        assert_eq!(
            parse_module_path(content),
            Some("github.com/foo/bar".to_string())
        );
    }

    #[test]
    fn read_version_from_comment() {
        let content = "// changelogs:version 1.2.3\nmodule github.com/foo/bar\n";
        assert_eq!(
            read_version_from_content(content),
            Some(Version::new(1, 2, 3))
        );
    }

    #[test]
    fn read_version_missing_comment_returns_none() {
        let content = "module github.com/foo/bar\n";
        assert_eq!(read_version_from_content(content), None);
    }

    #[test]
    fn read_version_handles_indented_comment() {
        let content = "  // changelogs:version 0.5.0\nmodule github.com/foo/bar\n";
        assert_eq!(
            read_version_from_content(content),
            Some(Version::new(0, 5, 0))
        );
    }

    #[test]
    fn upsert_inserts_when_missing() {
        let content = "module github.com/foo/bar\n\ngo 1.22\n";
        let updated = upsert_version_comment(content, &Version::new(1, 0, 0)).unwrap();
        assert!(updated.starts_with("// changelogs:version 1.0.0\nmodule github.com/foo/bar"));
        assert!(updated.ends_with('\n'));
    }

    #[test]
    fn upsert_replaces_when_present() {
        let content = "// changelogs:version 1.0.0\nmodule github.com/foo/bar\n";
        let updated = upsert_version_comment(content, &Version::new(2, 0, 0)).unwrap();
        assert!(updated.contains("// changelogs:version 2.0.0"));
        assert!(!updated.contains("1.0.0"));
    }

    #[test]
    fn upsert_errors_without_module_directive() {
        let content = "go 1.22\n";
        let err = upsert_version_comment(content, &Version::new(1, 0, 0)).unwrap_err();
        assert!(matches!(err, Error::GoModuleNotFound(_)));
    }

    #[test]
    fn parse_dependencies_block_form() {
        let content = "module github.com/foo/bar\n\nrequire (\n\tgithub.com/a/b v1.0.0\n\tgithub.com/c/d v2.0.0 // indirect\n)\n";
        let deps = parse_dependencies(content);
        assert_eq!(deps, vec!["github.com/a/b", "github.com/c/d"]);
    }

    #[test]
    fn parse_dependencies_single_line_form() {
        let content = "module github.com/foo/bar\n\nrequire github.com/a/b v1.0.0\n";
        let deps = parse_dependencies(content);
        assert_eq!(deps, vec!["github.com/a/b"]);
    }

    #[test]
    fn rewrite_require_block_updates_only_named_dep() {
        let content = "module github.com/foo/bar\n\nrequire (\n\tgithub.com/a/b v1.0.0\n\tgithub.com/c/d v2.0.0 // indirect\n)\n";
        let (updated, modified) =
            rewrite_require(content, "github.com/a/b", &Version::new(1, 5, 0));
        assert!(modified);
        assert!(updated.contains("github.com/a/b v1.5.0"));
        assert!(updated.contains("github.com/c/d v2.0.0 // indirect"));
    }

    #[test]
    fn rewrite_require_preserves_indirect_comment() {
        let content = "require (\n\tgithub.com/a/b v1.0.0 // indirect\n)\n";
        let (updated, modified) =
            rewrite_require(content, "github.com/a/b", &Version::new(1, 1, 0));
        assert!(modified);
        assert!(updated.contains("github.com/a/b v1.1.0 // indirect"));
    }

    #[test]
    fn rewrite_require_single_line_form() {
        let content = "module github.com/foo/bar\n\nrequire github.com/a/b v1.0.0\n";
        let (updated, modified) =
            rewrite_require(content, "github.com/a/b", &Version::new(2, 0, 0));
        assert!(modified);
        assert!(updated.contains("require github.com/a/b v2.0.0"));
    }

    #[test]
    fn rewrite_require_no_match_is_no_op() {
        let content = "require (\n\tgithub.com/a/b v1.0.0\n)\n";
        let (updated, modified) =
            rewrite_require(content, "github.com/zz/zz", &Version::new(9, 0, 0));
        assert!(!modified);
        assert_eq!(updated, content);
    }

    #[test]
    fn discover_single_module_with_comment_version() {
        let tmp = TempDir::new().unwrap();
        write_go_mod(
            tmp.path(),
            "// changelogs:version 0.7.0\nmodule github.com/foo/bar\n\ngo 1.22\n",
        );
        let pkgs = GoAdapter::discover(tmp.path()).unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "github.com/foo/bar");
        assert_eq!(pkgs[0].version, Version::new(0, 7, 0));
    }

    #[test]
    fn discover_falls_back_to_zero_version() {
        let tmp = TempDir::new().unwrap();
        write_go_mod(tmp.path(), "module github.com/foo/bar\n\ngo 1.22\n");
        let pkgs = GoAdapter::discover(tmp.path()).unwrap();
        assert_eq!(pkgs[0].version, Version::new(0, 0, 0));
    }

    #[test]
    fn discover_errors_without_go_mod() {
        let tmp = TempDir::new().unwrap();
        let err = GoAdapter::discover(tmp.path()).unwrap_err();
        assert!(matches!(err, Error::GoModuleNotFound(_)));
    }

    #[test]
    fn discover_errors_when_go_mod_lacks_module_directive() {
        let tmp = TempDir::new().unwrap();
        write_go_mod(tmp.path(), "go 1.22\n");
        let err = GoAdapter::discover(tmp.path()).unwrap_err();
        assert!(matches!(err, Error::GoModuleNotFound(_)));
    }

    #[test]
    fn read_and_write_version_round_trip() {
        let tmp = TempDir::new().unwrap();
        let manifest = write_go_mod(tmp.path(), "module github.com/foo/bar\n\ngo 1.22\n");

        // Initial read with no comment + no git tag → 0.0.0.
        let v = GoAdapter::read_version(&manifest).unwrap();
        assert_eq!(v, Version::new(0, 0, 0));

        GoAdapter::write_version(&manifest, &Version::new(0, 5, 0)).unwrap();
        let v = GoAdapter::read_version(&manifest).unwrap();
        assert_eq!(v, Version::new(0, 5, 0));

        GoAdapter::write_version(&manifest, &Version::new(1, 0, 0)).unwrap();
        let v = GoAdapter::read_version(&manifest).unwrap();
        assert_eq!(v, Version::new(1, 0, 0));
    }

    #[test]
    fn update_dependency_version_writes_file() {
        let tmp = TempDir::new().unwrap();
        let manifest = write_go_mod(
            tmp.path(),
            "module github.com/foo/bar\n\nrequire (\n\tgithub.com/a/b v1.0.0\n)\n",
        );
        let modified = GoAdapter::update_dependency_version(
            &manifest,
            "github.com/a/b",
            &Version::new(1, 1, 0),
        )
        .unwrap();
        assert!(modified);
        let content = fs::read_to_string(&manifest).unwrap();
        assert!(content.contains("github.com/a/b v1.1.0"));
    }

    #[test]
    fn publish_returns_success_without_doing_anything() {
        let tmp = TempDir::new().unwrap();
        write_go_mod(tmp.path(), "module github.com/foo/bar\n\ngo 1.22\n");
        let pkg = &GoAdapter::discover(tmp.path()).unwrap()[0];
        assert_eq!(
            GoAdapter::publish(pkg, false, None).unwrap(),
            PublishResult::Success
        );
        assert_eq!(
            GoAdapter::publish(pkg, true, None).unwrap(),
            PublishResult::Success
        );
    }

    #[test]
    fn is_published_skips_bootstrap_v0_0_0() {
        // v0.0.0 is the implicit baseline `discover` returns when there is no
        // version comment and no git tag. `is_published` must report it as
        // already published so the publisher does not bootstrap-tag the repo
        // on every push to the release branch. This case is short-circuited
        // before any network call, so it's safe to run offline.
        assert!(GoAdapter::is_published("github.com/foo/bar", &Version::new(0, 0, 0)).unwrap());
    }

    #[test]
    fn tag_name_format_for_go() {
        let pkg = Package {
            name: "github.com/foo/bar".to_string(),
            version: Version::new(1, 2, 3),
            path: PathBuf::from("/tmp/bar"),
            manifest_path: PathBuf::from("/tmp/bar/go.mod"),
            dependencies: vec![],
        };
        assert_eq!(GoAdapter::tag_name(&pkg), "v1.2.3");
    }

    #[test]
    fn escape_module_path_lowercases_with_bang() {
        assert_eq!(
            escape_module_path("github.com/Foo/Bar"),
            "github.com/!foo/!bar"
        );
        assert_eq!(
            escape_module_path("github.com/foo/bar"),
            "github.com/foo/bar"
        );
    }

    #[test]
    fn write_version_preserves_other_lines() {
        let tmp = TempDir::new().unwrap();
        let manifest = write_go_mod(
            tmp.path(),
            "module github.com/foo/bar\n\ngo 1.22\n\nrequire github.com/a/b v1.0.0\n",
        );
        GoAdapter::write_version(&manifest, &Version::new(1, 0, 0)).unwrap();
        let content = fs::read_to_string(&manifest).unwrap();
        assert!(content.contains("// changelogs:version 1.0.0"));
        assert!(content.contains("module github.com/foo/bar"));
        assert!(content.contains("go 1.22"));
        assert!(content.contains("require github.com/a/b v1.0.0"));
    }
}
