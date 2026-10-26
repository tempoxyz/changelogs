//! Contract tests for `--json`: the GitHub Action parses these documents,
//! so their shape must stay stable.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn write_crate(dir: &Path, name: &str, version: &str, extra: &str) {
    write(
        &dir.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\n{extra}"
        ),
    );
    write(&dir.join("src/lib.rs"), "");
}

fn init_repo(root: &Path, config: &str) {
    write(&root.join(".changelog/config.toml"), config);
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "initial"]);
}

/// Serves 404 for every request, so crates.io lookups find nothing, offline.
fn empty_registry() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            let _ = write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
        }
    });
    url
}

fn changelogs(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_changelogs"))
        .args(args)
        .current_dir(root)
        .env("CHANGELOGS_CRATES_INDEX_URL", empty_registry())
        .env_remove("CARGO_REGISTRY_TOKEN")
        .env_remove("CARGO_REGISTRY_DEFAULT")
        .output()
        .unwrap()
}

/// Parses stdout, which must hold nothing but the JSON document.
fn json_stdout(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}):\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn per_crate_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        &root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/app\", \"crates/lib\"]\nresolver = \"2\"\n",
    );
    write_crate(&root.join("crates/app"), "app", "1.0.0-rc1", "");
    write_crate(&root.join("crates/lib"), "lib", "0.1.0", "");
    write(
        &root.join(".changelog/brave-new-thing.md"),
        "---\nlib: minor\n---\n\nAdd a thing.\n",
    );
    init_repo(root, "[changelog]\nformat = \"per-crate\"\n");
    dir
}

#[test]
fn version_dry_run_json_per_crate() {
    let dir = per_crate_workspace();

    let output = changelogs(dir.path(), &["version", "--dry-run", "--json"]);

    assert!(output.status.success());
    let json = json_stdout(&output);
    assert_eq!(
        json,
        json!({
            "format": "per-crate",
            "releases": [{
                "name": "lib",
                "old_version": "0.1.0",
                "new_version": "0.2.0",
                "bump": "minor",
                "is_promotion": false,
                "changelog_path": "crates/lib/CHANGELOG.md",
                "notes": "## `lib@0.2.0`\n\n### Minor Changes\n\n- Add a thing.\n\n",
            }],
            "unified_version": null,
        })
    );
    // Human progress goes to stderr, and a dry run changes nothing.
    assert!(String::from_utf8_lossy(&output.stderr).contains("lib 0.1.0 → 0.2.0"));
    assert!(!dir.path().join("crates/lib/CHANGELOG.md").exists());

    let output = changelogs(
        dir.path(),
        &["version", "--dry-run", "--prerelease", "rc", "--json"],
    );
    assert!(output.status.success());
    let json = json_stdout(&output);
    assert_eq!(json["releases"][0]["new_version"], "0.2.0-rc1");
    assert_eq!(json["releases"][0]["is_promotion"], false);
}

#[test]
fn version_dry_run_json_reports_promotions() {
    let dir = per_crate_workspace();
    std::fs::remove_file(dir.path().join(".changelog/brave-new-thing.md")).unwrap();

    let output = changelogs(dir.path(), &["version", "--dry-run", "--json"]);
    assert!(output.status.success());
    let json = json_stdout(&output);
    assert_eq!(json["releases"].as_array().unwrap().len(), 1);
    assert_eq!(json["releases"][0]["name"], "app");
    assert_eq!(json["releases"][0]["new_version"], "1.0.0");
    assert_eq!(json["releases"][0]["is_promotion"], true);
    assert_eq!(json["releases"][0]["notes"], "");
}

#[test]
fn version_json_notes_match_notes_command() {
    let dir = per_crate_workspace();

    let output = changelogs(dir.path(), &["version", "--json"]);
    assert!(output.status.success());
    let json = json_stdout(&output);
    let notes = json["releases"][0]["notes"].as_str().unwrap();
    let changelog = std::fs::read_to_string(dir.path().join("crates/lib/CHANGELOG.md")).unwrap();
    assert!(changelog.contains(notes), "{changelog}");

    let output = changelogs(dir.path(), &["notes", "lib@0.2.0"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "### Minor Changes\n\n- Add a thing.\n"
    );
    assert!(
        !changelogs(dir.path(), &["notes", "lib@9.9.9"])
            .status
            .success()
    );
}

#[test]
fn version_dry_run_json_root() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write_crate(
        root,
        "product",
        "1.0.0",
        "\n[workspace]\nmembers = [\".\", \"helper\"]\n",
    );
    write_crate(&root.join("helper"), "helper", "1.0.0", "");
    write(
        &root.join(".changelog/calm-root-change.md"),
        "---\nproduct: minor\nhelper: minor\n---\n\nShip it.\n",
    );
    init_repo(
        root,
        "[changelog]\nformat = \"root\"\n\n[[fixed]]\nmembers = [\"product\", \"helper\"]\n",
    );

    let output = changelogs(root, &["version", "--dry-run", "--json"]);

    assert!(output.status.success());
    let json = json_stdout(&output);
    assert_eq!(json["format"], "root");
    assert_eq!(json["unified_version"], "1.1.0");
    let releases = json["releases"].as_array().unwrap();
    assert_eq!(releases.len(), 2);
    for release in releases {
        assert_eq!(release["new_version"], "1.1.0");
        assert_eq!(release["changelog_path"], "CHANGELOG.md");
        let notes = release["notes"].as_str().unwrap();
        assert!(notes.starts_with("## 1.1.0 ("), "{notes}");
        assert!(notes.contains("- Ship it."), "{notes}");
    }
}

#[test]
fn version_json_without_changes_prints_empty_plan() {
    let dir = TempDir::new().unwrap();
    write_crate(dir.path(), "solo", "1.0.0", "");
    init_repo(dir.path(), "[changelog]\nformat = \"per-crate\"\n");

    let output = changelogs(dir.path(), &["version", "--dry-run", "--json"]);

    assert!(output.status.success());
    assert_eq!(
        json_stdout(&output),
        json!({"format": "per-crate", "releases": [], "unified_version": null})
    );
}

/// Without a registry token, `publish` only creates tags.
#[test]
fn publish_json_tags_only() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        &root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/app\", \"crates/tool\"]\nresolver = \"2\"\n",
    );
    write_crate(&root.join("crates/app"), "app", "1.0.0", "");
    write_crate(
        &root.join("crates/tool"),
        "tool",
        "0.3.0",
        "publish = false\n",
    );
    init_repo(
        root,
        "private = [\"tool\"]\n[changelog]\nformat = \"per-crate\"\n",
    );

    let output = changelogs(root, &["publish", "--json"]);

    assert!(output.status.success());
    assert_eq!(
        json_stdout(&output),
        json!({
            "published": [],
            "skipped": [
                {"name": "app", "version": "1.0.0", "tag": "app@1.0.0", "reason": "no_token"},
                {"name": "tool", "version": "0.3.0", "tag": "tool@0.3.0", "reason": "not_publishable"},
            ],
            "failed": [],
            "tags_created": ["app@1.0.0", "tool@0.3.0"],
        })
    );

    // A rerun finds everything released and still prints a document.
    let output = changelogs(root, &["publish", "--json"]);
    assert!(output.status.success());
    assert_eq!(json_stdout(&output)["tags_created"], json!([]));
}

/// A failed publish still prints the JSON document, then exits non-zero.
#[test]
fn publish_json_failure() {
    let dir = TempDir::new().unwrap();
    write_crate(dir.path(), "app", "1.0.0", "");
    init_repo(dir.path(), "[changelog]\nformat = \"per-crate\"\n");

    let output = Command::new(env!("CARGO_BIN_EXE_changelogs"))
        .args(["publish", "--json", "--tag", "missing-registry"])
        .current_dir(dir.path())
        .env("CHANGELOGS_CRATES_INDEX_URL", empty_registry())
        .env("CARGO_REGISTRY_TOKEN", "dummy")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json = json_stdout(&output);
    assert_eq!(json["published"], json!([]));
    assert_eq!(json["tags_created"], json!([]));
    assert_eq!(json["failed"][0]["name"], "app");
    assert_eq!(json["failed"][0]["version"], "1.0.0");
    assert!(json["failed"][0]["error"].is_string());
}
