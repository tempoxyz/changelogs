use crate::BumpType;
use crate::error::{Error, Result};
use rand::RngExt;

use std::path::Path;

const ADJECTIVES: &[&str] = &[
    "brave", "calm", "dark", "eager", "fair", "gentle", "happy", "icy", "jolly", "keen", "lively",
    "merry", "nice", "odd", "proud", "quick", "rare", "shy", "tall", "unique", "vast", "warm",
    "young", "zesty", "bold", "cool", "dry", "easy", "fast", "good", "hot", "kind", "lazy", "mild",
    "neat", "old", "plain", "quiet", "rich", "safe", "tidy", "ugly", "vain", "weak", "aged", "big",
    "cute", "dull", "evil", "fine",
];

const NOUNS: &[&str] = &[
    "lions", "bears", "wolves", "eagles", "hawks", "foxes", "deer", "owls", "cats", "dogs",
    "birds", "fish", "frogs", "bees", "ants", "mice", "rats", "bats", "crows", "doves", "ducks",
    "geese", "hens", "pigs", "cows", "goats", "sheep", "horses", "mules", "donkeys", "tigers",
    "pandas", "koalas", "seals", "whales", "sharks", "crabs", "clams", "snails", "slugs", "trees",
    "rocks", "waves", "winds", "clouds", "stars", "moons", "suns", "hills", "lakes",
];

const VERBS: &[&str] = &[
    "dance", "sing", "jump", "run", "walk", "swim", "fly", "crawl", "climb", "slide", "roll",
    "spin", "twist", "shake", "wave", "bow", "nod", "wink", "smile", "laugh", "cry", "shout",
    "whisper", "hum", "buzz", "roar", "growl", "bark", "meow", "chirp", "play", "rest", "sleep",
    "wake", "eat", "drink", "cook", "bake", "read", "write", "draw", "paint", "build", "break",
    "fix", "clean", "wash", "dry", "fold", "pack",
];

pub fn generate_id() -> String {
    let mut rng = rand::rng();
    let adj = ADJECTIVES[rng.random_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.random_range(0..NOUNS.len())];
    let verb = VERBS[rng.random_range(0..VERBS.len())];
    format!("{}-{}-{}", adj, noun, verb)
}

#[derive(Debug, Clone)]
pub struct Changelog {
    pub id: String,
    pub summary: String,
    pub releases: Vec<Release>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub package: String,
    pub bump: BumpType,
}

pub fn parse(id: &str, content: &str) -> Result<Changelog> {
    let content = content.trim();

    if !content.starts_with("---") {
        return Err(Error::ChangelogParse(
            id.to_string(),
            "missing frontmatter".to_string(),
        ));
    }

    let rest = &content[3..];
    let end = rest.find("---").ok_or_else(|| {
        Error::ChangelogParse(id.to_string(), "missing frontmatter end".to_string())
    })?;

    let frontmatter = &rest[..end].trim();
    let summary = rest[end + 3..].trim().to_string();

    let frontmatter_value: serde_yaml::Value = serde_yaml::from_str(frontmatter)?;

    let commit = frontmatter_value
        .get("commit")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut releases = Vec::new();
    if let serde_yaml::Value::Mapping(map) = frontmatter_value {
        for (key, value) in map {
            if let (serde_yaml::Value::String(package), serde_yaml::Value::String(bump_str)) =
                (key, value)
            {
                if package == "commit" {
                    continue;
                }
                let bump: BumpType = bump_str.parse().map_err(|_| {
                    Error::ChangelogParse(
                        id.to_string(),
                        format!("invalid bump type: {}", bump_str),
                    )
                })?;
                releases.push(Release { package, bump });
            }
        }
    }

    Ok(Changelog {
        id: id.to_string(),
        summary,
        releases,
        commit,
    })
}

pub fn serialize(changelog: &Changelog) -> String {
    let mut frontmatter = String::new();
    for release in &changelog.releases {
        frontmatter.push_str(&format!("{}: {}\n", release.package, release.bump));
    }

    format!("---\n{}---\n\n{}\n", frontmatter, changelog.summary)
}

pub struct CommitInfo {
    pub pr_number: Option<u32>,
    pub commit_sha: String,
    pub authors: Vec<String>,
}

pub fn get_commit_info(_changelog_dir: &Path, id: &str) -> Option<CommitInfo> {
    let file_path = format!(".changelog/{}.md", id);

    // Step 1: Find the commit that originally added the file
    let add_commit = find_add_commit(&file_path)?;

    // Step 2: Check if the add commit itself has a PR number (squash merge case)
    // For squash merges, the commit message contains "(#123)"
    let output = std::process::Command::new("git")
        .args(["log", "--format=%s", "-1", &add_commit])
        .output()
        .ok()?;

    let commit_message = String::from_utf8_lossy(&output.stdout);
    let authors = get_commit_authors(&file_path, &add_commit);

    if let Some(pr_number) = extract_pr_number(commit_message.trim()) {
        return Some(CommitInfo {
            pr_number: Some(pr_number),
            commit_sha: add_commit,
            authors,
        });
    }

    // Step 3: Look for merge commit (traditional merge case)
    // Find the first merge commit that contains the add commit
    let merge_output = std::process::Command::new("git")
        .args([
            "log",
            "--merges",
            "--ancestry-path",
            "--reverse",
            "--format=%H %s",
            &format!("{}..HEAD", add_commit),
        ])
        .output()
        .ok()?;

    let merge_stdout = String::from_utf8_lossy(&merge_output.stdout);

    if let Some(merge_line) = merge_stdout.lines().next() {
        let merge_line = merge_line.trim();
        if !merge_line.is_empty() {
            let parts: Vec<&str> = merge_line.splitn(2, ' ').collect();
            if parts.len() >= 2 {
                let commit_sha = parts[0].to_string();
                let commit_message = parts[1];
                if let Some(pr_number) = extract_pr_number(commit_message) {
                    return Some(CommitInfo {
                        pr_number: Some(pr_number),
                        commit_sha,
                        authors,
                    });
                }
            }
        }
    }

    // Fallback: no PR number found
    Some(CommitInfo {
        pr_number: None,
        commit_sha: add_commit,
        authors,
    })
}

fn find_add_commit(file_path: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args([
            "log",
            "--follow",
            "--diff-filter=A",
            "--format=%H",
            "-1",
            "--",
            file_path,
        ])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sha = stdout.trim();
    if sha.is_empty() {
        None
    } else {
        Some(sha.to_string())
    }
}

fn get_commit_authors(file_path: &str, add_commit: &str) -> Vec<String> {
    // Get authors from the add commit and any commits that touched the file
    // up to that point (for PRs with multiple commits before squash/merge)
    let output = std::process::Command::new("git")
        .args([
            "log",
            "--follow",
            "--format=%aN",
            &format!("{}^..{}", add_commit, add_commit),
            "--",
            file_path,
        ])
        .output()
        .ok();

    let Some(output) = output else {
        return Vec::new();
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut authors: Vec<String> = stdout
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // If no authors found with range, try just the add commit
    if authors.is_empty() {
        let fallback = std::process::Command::new("git")
            .args(["log", "--format=%aN", "-1", add_commit])
            .output()
            .ok();

        if let Some(fb_output) = fallback {
            let fb_stdout = String::from_utf8_lossy(&fb_output.stdout);
            let author = fb_stdout.trim();
            if !author.is_empty() {
                authors.push(author.to_string());
            }
        }
    }

    authors.sort();
    authors.dedup();
    authors
}

fn extract_pr_number(message: &str) -> Option<u32> {
    let re = regex::Regex::new(r"\(#(\d+)\)").ok()?;
    re.captures(message)
        .and_then(|cap| cap.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

pub fn read_all(changelog_dir: &Path) -> Result<Vec<Changelog>> {
    let mut changelogs = Vec::new();

    if !changelog_dir.exists() {
        return Ok(changelogs);
    }

    for entry in std::fs::read_dir(changelog_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "md") {
            let filename = path.file_stem().unwrap().to_string_lossy().to_string();

            if filename == "README" {
                continue;
            }

            let content = std::fs::read_to_string(&path)?;
            let changelog = parse(&filename, &content)?;
            changelogs.push(changelog);
        }
    }

    changelogs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(changelogs)
}

pub fn write(changelog_dir: &Path, changelog: &Changelog) -> Result<()> {
    let path = changelog_dir.join(format!("{}.md", changelog.id));
    let content = serialize(changelog);
    std::fs::write(path, content)?;
    Ok(())
}

pub fn delete(changelog_dir: &Path, id: &str) -> Result<()> {
    let path = changelog_dir.join(format!("{}.md", id));
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_changelog() {
        let content = r#"---
my-crate: minor
other-crate: patch
---

Added new feature X.

Fixed bug Y.
"#;

        let changelog = parse("test-changelog", content).unwrap();
        assert_eq!(changelog.id, "test-changelog");
        assert_eq!(changelog.releases.len(), 2);
        assert!(changelog.summary.contains("Added new feature X"));
    }

    #[test]
    fn test_serialize_changelog() {
        let changelog = Changelog {
            id: "test".to_string(),
            summary: "Test summary".to_string(),
            releases: vec![Release {
                package: "my-crate".to_string(),
                bump: BumpType::Minor,
            }],
            commit: None,
        };

        let serialized = serialize(&changelog);
        assert!(serialized.contains("my-crate: minor"));
        assert!(serialized.contains("Test summary"));
    }

    #[test]
    fn test_generate_id() {
        let id = generate_id();
        let parts: Vec<_> = id.split('-').collect();
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn test_parse_empty_content() {
        let err = parse("test-id", "").unwrap_err();
        match err {
            Error::ChangelogParse(id, msg) => {
                assert_eq!(id, "test-id");
                assert!(msg.contains("missing frontmatter"));
            }
            _ => panic!("expected ChangelogParse error"),
        }
    }

    #[test]
    fn test_parse_missing_frontmatter_close() {
        let err = parse("test-id", "---\nfoo: bar\n").unwrap_err();
        match err {
            Error::ChangelogParse(id, msg) => {
                assert_eq!(id, "test-id");
                assert!(msg.contains("missing frontmatter end"));
            }
            _ => panic!("expected ChangelogParse error"),
        }
    }

    #[test]
    fn test_parse_non_string_yaml_values() {
        let content = "---\nmy-crate: 123\n---\nsummary";
        let result = parse("test-id", content);
        assert!(result.is_ok());
        let changelog = result.unwrap();
        assert!(changelog.releases.is_empty());
    }

    #[test]
    fn test_parse_with_commit_field() {
        let content = "---\ncommit: abc123\nmy-crate: minor\n---\nsummary";
        let changelog = parse("test-id", content).unwrap();
        assert_eq!(changelog.commit, Some("abc123".to_string()));
        assert_eq!(changelog.releases.len(), 1);
        assert_eq!(changelog.releases[0].package, "my-crate");
    }

    #[test]
    fn test_parse_no_releases() {
        let content = "---\ncommit: abc123\n---\nsummary";
        let changelog = parse("test-id", content).unwrap();
        assert_eq!(changelog.commit, Some("abc123".to_string()));
        assert!(changelog.releases.is_empty());
    }

    #[test]
    fn test_extract_pr_number_squash_merge() {
        assert_eq!(extract_pr_number("feat: add feature (#42)"), Some(42));
    }

    #[test]
    fn test_extract_pr_number_merge_commit() {
        assert_eq!(
            extract_pr_number("Merge pull request (#123) from branch"),
            Some(123)
        );
    }

    #[test]
    fn test_extract_pr_number_no_pr() {
        assert_eq!(extract_pr_number("regular commit message"), None);
    }

    #[test]
    fn test_extract_pr_number_multiple_takes_first() {
        assert_eq!(extract_pr_number("fix (#1) and (#2)"), Some(1));
    }

    #[test]
    fn test_read_all_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("beta-entry.md"),
            "---\npkg-a: minor\n---\n\nBeta summary\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("alpha-entry.md"),
            "---\npkg-b: patch\n---\n\nAlpha summary\n",
        )
        .unwrap();

        let changelogs = read_all(dir.path()).unwrap();
        assert_eq!(changelogs.len(), 2);
        assert_eq!(changelogs[0].id, "alpha-entry");
        assert_eq!(changelogs[1].id, "beta-entry");
    }

    #[test]
    fn test_read_all_skips_readme() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Changelogs").unwrap();
        std::fs::write(
            dir.path().join("real-entry.md"),
            "---\npkg: minor\n---\n\nReal entry\n",
        )
        .unwrap();

        let changelogs = read_all(dir.path()).unwrap();
        assert_eq!(changelogs.len(), 1);
        assert_eq!(changelogs[0].id, "real-entry");
    }

    #[test]
    fn test_read_all_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let changelogs = read_all(dir.path()).unwrap();
        assert!(changelogs.is_empty());
    }

    #[test]
    fn test_read_all_nonexistent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nonexistent = dir.path().join("does-not-exist");
        let changelogs = read_all(&nonexistent).unwrap();
        assert!(changelogs.is_empty());
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let changelog = Changelog {
            id: "roundtrip-test".to_string(),
            summary: "Roundtrip summary".to_string(),
            releases: vec![Release {
                package: "my-crate".to_string(),
                bump: BumpType::Minor,
            }],
            commit: None,
        };

        write(dir.path(), &changelog).unwrap();
        let changelogs = read_all(dir.path()).unwrap();
        assert_eq!(changelogs.len(), 1);
        assert_eq!(changelogs[0].id, "roundtrip-test");
        assert_eq!(changelogs[0].summary, "Roundtrip summary");
        assert_eq!(changelogs[0].releases.len(), 1);
        assert_eq!(changelogs[0].releases[0].package, "my-crate");
        assert_eq!(changelogs[0].releases[0].bump, BumpType::Minor);
    }

    #[test]
    fn test_delete_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("to-delete.md");
        std::fs::write(&path, "---\npkg: patch\n---\n\nDelete me\n").unwrap();
        assert!(path.exists());

        delete(dir.path(), "to-delete").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_delete_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(delete(dir.path(), "no-such-id").is_ok());
    }
}
