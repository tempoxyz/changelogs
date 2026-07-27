use anyhow::Result;
use changelogs::Ecosystem;
use changelogs::changelog_entry;
use changelogs::config::Config;
use changelogs::error::Error;
use changelogs::workspace::Workspace;
use console::style;
use std::process::Command;

enum CheckResult {
    Pass(String),
    Fail(String),
}

impl CheckResult {
    fn print(&self) {
        match self {
            CheckResult::Pass(msg) => println!("  {} {msg}", style("✓").green()),
            CheckResult::Fail(msg) => println!("  {} {msg}", style("✗").red()),
        }
    }

    fn is_pass(&self) -> bool {
        matches!(self, CheckResult::Pass(_))
    }
}

fn check_workspace(ecosystem: Option<Ecosystem>) -> (CheckResult, Option<Workspace>) {
    match Workspace::discover_with_ecosystem(ecosystem) {
        Ok(ws) => {
            let msg = format!("Workspace detected ({})", style(ws.root.display()).dim());
            (CheckResult::Pass(msg), Some(ws))
        }
        Err(Error::ConfigParse(error)) => (
            CheckResult::Fail(format!("Config parse failed: {error}")),
            None,
        ),
        Err(Error::UnknownPrivatePackage(package)) => (
            CheckResult::Fail(format!(
                "Private list references unknown package: {package}"
            )),
            None,
        ),
        Err(e) => (
            CheckResult::Fail(format!("Workspace detection failed: {e}")),
            None,
        ),
    }
}

fn check_initialized(workspace: &Workspace) -> CheckResult {
    if workspace.is_initialized() {
        CheckResult::Pass("Changelog directory initialized".into())
    } else {
        CheckResult::Fail(format!(
            "Changelog directory not initialized — run {}",
            style("changelogs init").cyan()
        ))
    }
}

fn check_config(changelog_dir: &std::path::Path) -> (CheckResult, Option<Config>) {
    match Config::load(changelog_dir) {
        Ok(c) => (CheckResult::Pass("Config is valid".into()), Some(c)),
        Err(e) => (CheckResult::Fail(format!("Config parse failed: {e}")), None),
    }
}

fn check_fixed_groups(config: &Config, package_names: &[&str]) -> Vec<CheckResult> {
    config
        .fixed
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let invalid: Vec<_> = group
                .members
                .iter()
                .filter(|m| !package_names.contains(&m.as_str()))
                .collect();
            if invalid.is_empty() {
                CheckResult::Pass(format!("Fixed group {} — all members valid", i + 1))
            } else {
                CheckResult::Fail(format!(
                    "Fixed group {} references unknown packages: {}",
                    i + 1,
                    invalid
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        })
        .collect()
}

fn check_linked_groups(config: &Config, package_names: &[&str]) -> Vec<CheckResult> {
    config
        .linked
        .iter()
        .enumerate()
        .map(|(i, group)| {
            let invalid: Vec<_> = group
                .members
                .iter()
                .filter(|m| !package_names.contains(&m.as_str()))
                .collect();
            if invalid.is_empty() {
                CheckResult::Pass(format!("Linked group {} — all members valid", i + 1))
            } else {
                CheckResult::Fail(format!(
                    "Linked group {} references unknown packages: {}",
                    i + 1,
                    invalid
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        })
        .collect()
}

fn check_ignore_list(config: &Config, package_names: &[&str]) -> CheckResult {
    let invalid: Vec<_> = config
        .ignore
        .iter()
        .filter(|m| !package_names.contains(&m.as_str()))
        .collect();
    if invalid.is_empty() {
        CheckResult::Pass("Ignore list — all entries valid".into())
    } else {
        CheckResult::Fail(format!(
            "Ignore list references unknown packages: {}",
            invalid
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

fn check_private_list(config: &Config, package_names: &[&str]) -> CheckResult {
    let invalid: Vec<_> = config
        .private
        .iter()
        .filter(|member| !package_names.contains(&member.as_str()))
        .collect();
    if invalid.is_empty() {
        CheckResult::Pass("Private list: all entries valid".into())
    } else {
        CheckResult::Fail(format!(
            "Private list references unknown packages: {}",
            invalid
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

fn check_pending_changelogs(
    changelog_dir: &std::path::Path,
    package_names: &[&str],
) -> CheckResult {
    match changelog_entry::read_all(changelog_dir) {
        Ok(changelogs) => {
            let mut invalid_refs: Vec<String> = Vec::new();
            for changelog in &changelogs {
                for release in &changelog.releases {
                    if !package_names.contains(&release.package.as_str()) {
                        invalid_refs.push(format!("'{}' in {}", release.package, changelog.id));
                    }
                }
            }
            if invalid_refs.is_empty() {
                CheckResult::Pass("Pending changelogs — all package references valid".into())
            } else {
                let details = invalid_refs
                    .iter()
                    .map(|r| format!("      {}", style(r).dim()))
                    .collect::<Vec<_>>()
                    .join("\n");
                CheckResult::Fail(format!(
                    "Pending changelogs reference unknown packages:\n{details}"
                ))
            }
        }
        Err(e) => CheckResult::Fail(format!("Failed to read changelogs: {e}")),
    }
}

fn check_git_remote() -> CheckResult {
    let remote_ok = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);

    if remote_ok {
        CheckResult::Pass("Git remote detected".into())
    } else {
        CheckResult::Fail(
            "Git remote not detected — changelog links will not include PR/commit references"
                .into(),
        )
    }
}

fn run_checks(results: &mut Vec<CheckResult>, checks: Vec<CheckResult>) -> bool {
    let all_passed = checks.iter().all(|r| r.is_pass());
    results.extend(checks);
    all_passed
}

pub fn run(ecosystem: Option<Ecosystem>) -> Result<()> {
    println!("{} Running diagnostics...\n", style("→").blue().bold());

    let mut results: Vec<CheckResult> = Vec::new();

    let (ws_check, workspace) = check_workspace(ecosystem);
    if !run_checks(&mut results, vec![ws_check]) {
        print_results(&results);
        return Ok(());
    }
    let workspace = workspace.unwrap();

    if !run_checks(&mut results, vec![check_initialized(&workspace)]) {
        print_results(&results);
        return Ok(());
    }

    let changelog_dir = workspace.changelog_dir();
    let package_names: Vec<&str> = workspace.package_names();

    let (config_check, config) = check_config(&changelog_dir);
    if !run_checks(&mut results, vec![config_check]) {
        print_results(&results);
        return Ok(());
    }
    let config = config.unwrap();

    run_checks(&mut results, check_fixed_groups(&config, &package_names));
    run_checks(&mut results, check_linked_groups(&config, &package_names));
    run_checks(
        &mut results,
        vec![check_ignore_list(&config, &package_names)],
    );
    run_checks(
        &mut results,
        vec![check_private_list(&config, &package_names)],
    );
    run_checks(
        &mut results,
        vec![check_pending_changelogs(&changelog_dir, &package_names)],
    );
    run_checks(&mut results, vec![check_git_remote()]);

    print_results(&results);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_check_workspace_missing() {
        let (result, ws) = check_workspace(None);
        assert!(!result.is_pass() || ws.is_some());
    }

    fn fake_workspace(root: &std::path::Path) -> Workspace {
        Workspace {
            root: root.to_path_buf(),
            changelog_dir: root.join(".changelog"),
            packages: vec![],
            ecosystem: changelogs::Ecosystem::Rust,
        }
    }

    #[test]
    fn test_check_initialized_false() {
        let temp = TempDir::new().unwrap();
        let ws = fake_workspace(temp.path());
        let result = check_initialized(&ws);
        assert!(!result.is_pass());
    }

    #[test]
    fn test_check_initialized_true() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".changelog")).unwrap();
        let ws = fake_workspace(temp.path());
        let result = check_initialized(&ws);
        assert!(result.is_pass());
    }

    #[test]
    fn test_check_config_defaults_valid() {
        let temp = TempDir::new().unwrap();
        let (result, config) = check_config(temp.path());
        assert!(result.is_pass());
        assert!(config.is_some());
    }

    #[test]
    fn test_check_ignore_list_all_valid() {
        let config = Config {
            ignore: vec!["pkg-a".into()],
            ..Default::default()
        };
        let result = check_ignore_list(&config, &["pkg-a", "pkg-b"]);
        assert!(result.is_pass());
    }

    #[test]
    fn test_check_ignore_list_invalid() {
        let config = Config {
            ignore: vec!["pkg-missing".into()],
            ..Default::default()
        };
        let result = check_ignore_list(&config, &["pkg-a"]);
        assert!(!result.is_pass());
    }

    #[test]
    fn test_check_private_list_all_valid() {
        let config = Config {
            private: vec!["pkg-a".into()],
            ..Default::default()
        };
        let result = check_private_list(&config, &["pkg-a", "pkg-b"]);
        assert!(result.is_pass());
    }

    #[test]
    fn test_check_private_list_invalid() {
        let config = Config {
            private: vec!["pkg-missing".into()],
            ..Default::default()
        };
        let result = check_private_list(&config, &["pkg-a"]);
        assert!(!result.is_pass());
    }

    #[test]
    fn test_check_git_remote_in_non_git_dir() {
        let result = check_git_remote();
        assert!(result.is_pass() || !result.is_pass());
    }
}

fn print_results(results: &[CheckResult]) {
    for result in results {
        result.print();
    }

    let passed = results.iter().filter(|r| r.is_pass()).count();
    let failed = results.len() - passed;

    println!();
    if failed > 0 {
        println!(
            "{} {passed} passed, {failed} failed",
            style("✗").red().bold()
        );
    } else {
        println!("{} All {passed} checks passed", style("✓").green().bold());
    }
}
