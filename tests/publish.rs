use changelogs::ecosystems::{self, Ecosystem};
use changelogs::{Package, PublishResult, SkipReason};
use semver::Version;
use serial_test::serial;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn rust_package(dir: &std::path::Path) -> Package {
    let manifest = dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"my-binary\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    Package {
        name: "my-binary".to_string(),
        version: Version::new(1, 0, 0),
        path: dir.to_path_buf(),
        manifest_path: manifest,
        dependencies: vec![],
        publishable: true,
    }
}

/// Rust: absent `CARGO_REGISTRY_TOKEN` → `Skipped(NoToken)`, not an error.
///
/// Binary-only projects (e.g. foundry-rs/foundry) skip crates.io but still
/// want git tags and GitHub releases. The publish step must succeed so the
/// downstream tagging steps can run.
#[test]
#[serial]
fn rust_publish_without_token_skips_silently() {
    let dir = TempDir::new().unwrap();
    let pkg = rust_package(dir.path());

    // SAFETY: single-threaded test; no other test races on this env var.
    unsafe {
        std::env::remove_var("CARGO_REGISTRY_TOKEN");
    }

    let result = ecosystems::publish(Ecosystem::Rust, &pkg, false, None)
        .expect("publish must not error when no registry token is configured");

    assert_eq!(
        result,
        PublishResult::Skipped(SkipReason::NoToken),
        "expected Skipped(NoToken) — git-tag creation must still proceed"
    );
}

/// Python: absent `TWINE_PASSWORD` → `Skipped(NoToken)`, not an error.
///
/// The action always sets `TWINE_USERNAME=__token__`, so the no-token
/// condition must be triggered by the absence of `TWINE_PASSWORD` alone.
#[test]
#[serial]
fn python_publish_without_token_skips_silently() {
    let dir = TempDir::new().unwrap();
    let manifest = dir.path().join("pyproject.toml");
    std::fs::write(
        &manifest,
        "[project]\nname = \"my-tool\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let pkg = Package {
        name: "my-tool".to_string(),
        version: Version::new(1, 0, 0),
        path: dir.path().to_path_buf(),
        manifest_path: manifest,
        dependencies: vec![],
        publishable: true,
    };

    // Mirror the action's real "no pypi-token" environment: TWINE_USERNAME is
    // always set to __token__ by the action; TWINE_PASSWORD is what's absent.
    // SAFETY: serialized via #[serial]; no concurrent env mutation.
    unsafe {
        std::env::remove_var("TWINE_PASSWORD");
        std::env::set_var("TWINE_USERNAME", "__token__");
    }

    let result = ecosystems::publish(Ecosystem::Python, &pkg, false, None)
        .expect("publish must not error when no registry token is configured");

    assert_eq!(
        result,
        PublishResult::Skipped(SkipReason::NoToken),
        "expected Skipped(NoToken) — git-tag creation must still proceed"
    );
}

/// Serves canned responses by request path on a local port (anything else is
/// 404), so registry lookups run offline.
fn mock_registry(routes: Vec<(&str, u16, &str)>) -> String {
    let routes: Vec<(String, u16, String)> = routes
        .into_iter()
        .map(|(path, status, body)| (path.to_string(), status, body.to_string()))
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut header = String::new();
            while reader.read_line(&mut header).unwrap() > 2 {
                header.clear();
            }
            let path = request_line.split_whitespace().nth(1).unwrap_or_default();
            let (status, body) = routes
                .iter()
                .find(|(route, _, _)| route == path)
                .map(|(_, status, body)| (*status, body.as_str()))
                .unwrap_or((404, ""));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap()
}

fn tags(root: &Path) -> Vec<String> {
    git(root, &["tag", "--list"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn write_crate(dir: &Path, name: &str, extra: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\nedition = \"2021\"\n{extra}"),
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rs"), "").unwrap();
}

fn init_repo(root: &Path, config: &str) {
    let changelog_dir = root.join(".changelog");
    std::fs::create_dir_all(&changelog_dir).unwrap();
    std::fs::write(changelog_dir.join("config.toml"), config).unwrap();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "initial"]);
}

/// Runs `changelogs publish` with the crates index at `index_url`. With
/// `fail`, publishing to crates.io fails offline: a token is set and the
/// registry name does not exist.
fn publish(root: &Path, index_url: &str, fail: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_changelogs"));
    cmd.arg("publish")
        .current_dir(root)
        .env("CHANGELOGS_CRATES_INDEX_URL", index_url)
        .env_remove("CARGO_REGISTRY_TOKEN")
        .env_remove("CARGO_REGISTRY_DEFAULT");
    if fail {
        cmd.args(["--tag", "missing-registry"])
            .env("CARGO_REGISTRY_TOKEN", "dummy");
    }
    cmd.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Rerunning `publish` after a release was tagged must succeed, not fail
/// with "tag already exists".
#[test]
fn publish_twice_succeeds() {
    let dir = TempDir::new().unwrap();
    write_crate(dir.path(), "my-binary", "");
    init_repo(dir.path(), "[changelog]\nformat = \"per-crate\"\n");
    let index = mock_registry(vec![]);

    let first = publish(dir.path(), &index, false);
    assert!(first.status.success(), "{}", stdout(&first));
    assert_eq!(tags(dir.path()), vec!["my-binary@1.0.0"]);

    let second = publish(dir.path(), &index, false);
    assert!(second.status.success(), "{}", stdout(&second));
    assert!(stdout(&second).contains("No unpublished packages found"));
    assert_eq!(tags(dir.path()), vec!["my-binary@1.0.0"]);
}

/// In root format a failed package withholds the release tag; a retry where
/// the package is already on crates.io creates it.
#[test]
fn retry_after_partial_failure_tags_release() {
    let dir = TempDir::new().unwrap();
    write_crate(
        dir.path(),
        "product",
        "\n[workspace]\nmembers = [\".\", \"helper\"]\n",
    );
    write_crate(&dir.path().join("helper"), "helper", "publish = false\n");
    init_repo(
        dir.path(),
        "private = [\"helper\"]\n[changelog]\nformat = \"root\"\n",
    );

    let first = publish(dir.path(), &mock_registry(vec![]), true);
    assert!(!first.status.success(), "{}", stdout(&first));
    assert!(
        stdout(&first).contains("Not tagging the release"),
        "{}",
        stdout(&first)
    );
    assert!(tags(dir.path()).is_empty());

    let published = r#"{"name":"product","vers":"1.0.0"}"#;
    let index = mock_registry(vec![("/pr/od/product", 200, published)]);
    let second = publish(dir.path(), &index, false);
    assert!(second.status.success(), "{}", stdout(&second));
    assert_eq!(tags(dir.path()), vec!["v1.0.0"]);
}

/// Dependencies publish first even when their path sorts last, and a failed
/// dependency skips its dependents.
#[test]
fn publish_follows_dependency_order() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/a-app\", \"crates/z-lib\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    write_crate(
        &dir.path().join("crates/a-app"),
        "a-app",
        "\n[dependencies]\nz-lib = { path = \"../z-lib\", version = \"1.0.0\" }\n",
    );
    write_crate(&dir.path().join("crates/z-lib"), "z-lib", "");
    init_repo(dir.path(), "[changelog]\nformat = \"per-crate\"\n");
    let index = mock_registry(vec![]);

    let failed = publish(dir.path(), &index, true);
    assert!(!failed.status.success());
    assert!(
        stdout(&failed).contains("a-app v1.0.0 ... ⊘ (skipped: dependency z-lib failed)"),
        "{}",
        stdout(&failed)
    );
    assert!(tags(dir.path()).is_empty());

    let output = publish(dir.path(), &index, false);
    assert!(output.status.success(), "{}", stdout(&output));
    let out = stdout(&output);
    assert!(out.find("z-lib v1.0.0").unwrap() < out.find("a-app v1.0.0").unwrap());
    assert_eq!(tags(dir.path()), vec!["a-app@1.0.0", "z-lib@1.0.0"]);
}

/// crates.io: only the exact version in the sparse index counts.
#[test]
#[serial]
fn rust_is_published_reads_sparse_index() {
    let index = concat!(
        r#"{"name":"my-binary","vers":"0.9.0"}"#,
        "\n",
        r#"{"name":"my-binary","vers":"1.0.0-rc.1"}"#,
        "\n"
    );
    let url = mock_registry(vec![
        ("/my/-b/my-binary", 200, index),
        ("/br/ok/broken", 500, ""),
    ]);
    // SAFETY: serialized via #[serial]; no concurrent env mutation.
    unsafe {
        std::env::set_var("CHANGELOGS_CRATES_INDEX_URL", &url);
    }

    let published = |name: &str, version: &str| {
        ecosystems::is_published(Ecosystem::Rust, name, &version.parse().unwrap())
    };
    assert!(published("my-binary", "0.9.0").unwrap());
    assert!(published("my-binary", "1.0.0-rc.1").unwrap());
    assert!(!published("my-binary", "1.0.0").unwrap());
    assert!(!published("unknown", "1.0.0").unwrap());
    assert!(published("broken", "1.0.0").is_err());

    unsafe {
        std::env::remove_var("CHANGELOGS_CRATES_INDEX_URL");
    }
}

/// PyPI: the exact-release endpoint is queried with the semver string, which
/// PyPI normalizes (`1.6.0-rc1` finds `1.6.0rc1`).
#[test]
#[serial]
fn python_is_published_queries_exact_release() {
    let url = mock_registry(vec![
        ("/pypi/my-tool/1.6.0-rc1/json", 200, "{}"),
        ("/pypi/broken/1.0.0/json", 500, ""),
    ]);
    // SAFETY: serialized via #[serial]; no concurrent env mutation.
    unsafe {
        std::env::set_var("CHANGELOGS_PYPI_URL", &url);
    }

    let published = |name: &str, version: &str| {
        ecosystems::is_published(Ecosystem::Python, name, &version.parse().unwrap())
    };
    assert!(published("My_Tool", "1.6.0-rc1").unwrap());
    assert!(!published("my-tool", "1.6.0").unwrap());
    assert!(published("broken", "1.0.0").is_err());

    unsafe {
        std::env::remove_var("CHANGELOGS_PYPI_URL");
    }
}
