//! Several workspaces released from one repository, each in its own
//! subdirectory with its own `.changelog/` and `tag_prefix`.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap()
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn tags(root: &Path) -> Vec<String> {
    git(root, &["tag", "--list"])
        .lines()
        .map(str::to_string)
        .collect()
}

/// Serves 404 for every request, so registry lookups find nothing, offline.
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

fn changelogs(dir: &Path, args: &[&str]) -> Output {
    let registry = empty_registry();
    Command::new(env!("CARGO_BIN_EXE_changelogs"))
        .args(args)
        .current_dir(dir)
        .env("CHANGELOGS_CRATES_INDEX_URL", &registry)
        .env("CHANGELOGS_GO_PROXY_URL", &registry)
        .env_remove("CARGO_REGISTRY_TOKEN")
        .env_remove("CARGO_REGISTRY_DEFAULT")
        .output()
        .unwrap()
}

fn json_stdout(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}):\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// A repository with a Rust crate in `rust/` and a Go module in `go/`, both
/// at version 1.0.0, each with its own prefix.
fn monorepo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        &root.join("rust/Cargo.toml"),
        "[package]\nname = \"sdk\"\nversion = \"1.0.0\"\nedition = \"2021\"\n",
    );
    write(&root.join("rust/src/lib.rs"), "");
    write(
        &root.join("rust/.changelog/config.toml"),
        "tag_prefix = \"rust/\"\n[changelog]\nformat = \"root\"\n",
    );
    write(
        &root.join("go/go.mod"),
        "// changelogs:version 1.0.0\nmodule github.com/example/sdk\n\ngo 1.22\n",
    );
    write(
        &root.join("go/.changelog/config.toml"),
        "tag_prefix = \"go/\"\n",
    );
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "initial"]);
    dir
}

#[test]
fn each_workspace_tags_its_own_releases() {
    let dir = monorepo();
    let root = dir.path();

    let rust = changelogs(&root.join("rust"), &["publish", "--json"]);
    assert!(
        rust.status.success(),
        "{}",
        String::from_utf8_lossy(&rust.stderr)
    );
    let report = json_stdout(&rust);
    assert_eq!(report["tags_created"], serde_json::json!(["rust/v1.0.0"]));
    assert_eq!(report["tag_prefix"], "rust/");
    assert_eq!(report["release_tags"], serde_json::json!(["rust/v1.0.0"]));

    // The same version in another workspace is not mistaken for released.
    let go = changelogs(&root.join("go"), &["publish", "--json"]);
    assert!(
        go.status.success(),
        "{}",
        String::from_utf8_lossy(&go.stderr)
    );
    assert_eq!(
        json_stdout(&go)["tags_created"],
        serde_json::json!(["go/v1.0.0"])
    );

    assert_eq!(tags(root), vec!["go/v1.0.0", "rust/v1.0.0"]);

    // A rerun finds both releases tagged.
    for sub in ["rust", "go"] {
        let again = changelogs(&root.join(sub), &["publish", "--json"]);
        assert!(again.status.success());
        assert_eq!(json_stdout(&again)["tags_created"], serde_json::json!([]));
    }
    assert_eq!(tags(root), vec!["go/v1.0.0", "rust/v1.0.0"]);
}

#[test]
fn version_json_reports_prefixed_unified_tag() {
    let dir = monorepo();
    let rust = dir.path().join("rust");
    write(
        &rust.join(".changelog/quick-fix.md"),
        "---\nsdk: patch\n---\n\nFixed a thing.\n",
    );

    let output = changelogs(&rust, &["version", "--dry-run", "--json"]);
    assert!(output.status.success());
    let json = json_stdout(&output);
    assert_eq!(json["unified_version"], "1.0.1");
    assert_eq!(json["unified_tag"], "rust/v1.0.1");
}

#[test]
fn prefixed_go_module_needs_version_comment() {
    let dir = monorepo();
    let go = dir.path().join("go");
    write(
        &go.join("go.mod"),
        "module github.com/example/sdk\n\ngo 1.22\n",
    );

    let output = changelogs(&go, &["publish"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("changelogs:version"), "{stderr}");
}
