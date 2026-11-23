use crate::ecosystems::Ecosystem;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ecosystem: Option<Ecosystem>,

    #[serde(default = "default_dependent_bump")]
    pub dependent_bump: DependentBump,

    #[serde(default)]
    pub changelog: ChangelogConfig,

    #[serde(default)]
    pub fixed: Vec<FixedGroup>,

    #[serde(default)]
    pub linked: Vec<LinkedGroup>,

    #[serde(default)]
    pub ignore: Vec<String>,

    #[serde(default)]
    pub private: Vec<String>,

    #[serde(default)]
    pub ai: AiConfig,

    /// Prepended to every release tag, such as `python/` for `python/v1.2.0`
    /// or `python/pkg@1.2.0`. Lets several workspaces release from one
    /// repository without their tags colliding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConfig {
    pub command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DependentBump {
    #[default]
    Patch,
    Minor,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogConfig {
    #[serde(default = "default_changelog_format")]
    pub format: ChangelogFormat,
}

impl Default for ChangelogConfig {
    fn default() -> Self {
        Self {
            format: default_changelog_format(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ChangelogFormat {
    #[default]
    PerCrate,
    Root,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixedGroup {
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedGroup {
    pub members: Vec<String>,
}

fn default_dependent_bump() -> DependentBump {
    DependentBump::Patch
}

fn default_changelog_format() -> ChangelogFormat {
    ChangelogFormat::PerCrate
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ecosystem: None,
            dependent_bump: default_dependent_bump(),
            changelog: ChangelogConfig::default(),
            fixed: Vec::new(),
            linked: Vec::new(),
            ignore: Vec::new(),
            private: Vec::new(),
            ai: AiConfig::default(),
            tag_prefix: None,
        }
    }
}

impl Config {
    pub fn load(changelog_dir: &Path) -> Result<Self> {
        let config_path = changelog_dir.join("config.toml");

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: Config =
            toml::from_str(&content).map_err(|e| Error::ConfigParse(e.to_string()))?;
        config.validate()?;

        Ok(config)
    }

    /// Rejects a `tag_prefix` that would not make a valid git tag name, before
    /// anything is published.
    pub fn validate(&self) -> Result<()> {
        if let Some(prefix) = &self.tag_prefix {
            let components: Vec<&str> = prefix.split('/').collect();
            let (last, complete) = components.split_last().expect("split yields one");
            // Git refname rules: no component starts with a dot or ends with
            // `.lock`; the last one continues into the version, so only its
            // start is known.
            let invalid = prefix.is_empty()
                || prefix.starts_with('/')
                || prefix.starts_with('-')
                || prefix.contains("..")
                || prefix.contains("//")
                || prefix.contains("@{")
                || prefix
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
                || complete
                    .iter()
                    .any(|part| part.starts_with('.') || part.ends_with(".lock"))
                || last.starts_with('.');
            if invalid {
                return Err(Error::InvalidTagPrefix(prefix.clone()));
            }
        }
        Ok(())
    }

    /// `tag` with the configured `tag_prefix` in front of it.
    pub fn prefixed_tag(&self, tag: &str) -> String {
        format!("{}{tag}", self.tag_prefix.as_deref().unwrap_or(""))
    }

    pub fn save(&self, changelog_dir: &Path) -> Result<()> {
        let config_path = changelog_dir.join("config.toml");
        let content =
            toml::to_string_pretty(self).map_err(|e| Error::ConfigParse(e.to_string()))?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    pub fn default_toml() -> &'static str {
        r#"# Ecosystem: "rust" | "python" | "go" | "swift" (auto-detected if not specified)
# ecosystem = "rust"

# How to bump packages that depend on changed packages
# "patch" | "minor" | "none"
dependent_bump = "patch"

# Packages to ignore
ignore = []

# Prefix for every release tag, for several workspaces in one repository
# tag_prefix = "python/"   # tags python/v1.2.0 (root format) or python/pkg@1.2.0

# Private Rust packages to version and tag without publishing
private = []

# Fixed groups: all packages always share the same version
# [[fixed]]
# members = ["package-a", "package-b"]

# Linked groups: versions sync when released together
# [[linked]]
# members = ["sdk-core", "sdk-macros"]

[changelog]
# "per-crate" - CHANGELOG.md in each package
# "root" - Single CHANGELOG.md at workspace root
format = "per-crate"

# AI-assisted changelog generation
# [ai]
# command = "amp ask"  # or "gh copilot suggest -t shell"
"#
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_missing_file_returns_default() {
        let dir = TempDir::new().unwrap();
        let config = Config::load(dir.path()).unwrap();

        assert_eq!(config.dependent_bump, DependentBump::Patch);
        assert_eq!(config.changelog.format, ChangelogFormat::PerCrate);
        assert!(config.ecosystem.is_none());
        assert!(config.fixed.is_empty());
        assert!(config.linked.is_empty());
        assert!(config.ignore.is_empty());
        assert!(config.private.is_empty());
    }

    #[test]
    fn test_save_then_load_roundtrip() {
        let dir = TempDir::new().unwrap();

        let config = Config {
            ecosystem: None,
            dependent_bump: DependentBump::Minor,
            changelog: ChangelogConfig {
                format: ChangelogFormat::Root,
            },
            fixed: vec![FixedGroup {
                members: vec!["a".into(), "b".into()],
            }],
            linked: vec![LinkedGroup {
                members: vec!["x".into(), "y".into()],
            }],
            ignore: vec!["foo".into()],
            private: vec!["private-tool".into()],
            ai: AiConfig {
                command: Some("test-cmd".into()),
            },
            tag_prefix: Some("python/".into()),
        };

        config.save(dir.path()).unwrap();
        let loaded = Config::load(dir.path()).unwrap();

        assert_eq!(loaded.dependent_bump, DependentBump::Minor);
        assert_eq!(loaded.changelog.format, ChangelogFormat::Root);
        assert_eq!(loaded.fixed.len(), 1);
        assert_eq!(loaded.fixed[0].members, vec!["a", "b"]);
        assert_eq!(loaded.linked.len(), 1);
        assert_eq!(loaded.linked[0].members, vec!["x", "y"]);
        assert_eq!(loaded.ignore, vec!["foo"]);
        assert_eq!(loaded.private, vec!["private-tool"]);
        assert_eq!(loaded.ai.command.as_deref(), Some("test-cmd"));
        assert_eq!(loaded.tag_prefix.as_deref(), Some("python/"));
    }

    #[test]
    fn test_tag_prefix_prefixes_tags() {
        let config = Config {
            tag_prefix: Some("rust/".into()),
            ..Default::default()
        };
        assert_eq!(config.prefixed_tag("v1.2.0"), "rust/v1.2.0");
        assert_eq!(Config::default().prefixed_tag("v1.2.0"), "v1.2.0");
    }

    #[test]
    fn test_invalid_tag_prefix_is_rejected() {
        for prefix in [
            "",
            "/python/",
            "py thon/",
            "a..b/",
            "-x/",
            "a:b/",
            "a//b",
            ".sdk/",
            "sdk.lock/",
            "sdk/.go/",
            "sdk/.",
        ] {
            let dir = TempDir::new().unwrap();
            std::fs::write(
                dir.path().join("config.toml"),
                format!("tag_prefix = {prefix:?}\n"),
            )
            .unwrap();
            assert!(
                matches!(Config::load(dir.path()), Err(Error::InvalidTagPrefix(_))),
                "{prefix:?} should be rejected"
            );
        }
        for prefix in [
            "python/",
            "sdk-",
            "go/v2-",
            "sdk/go/",
            "sdk.v1/",
            "sdk/lock-",
        ] {
            let dir = TempDir::new().unwrap();
            std::fs::write(
                dir.path().join("config.toml"),
                format!("tag_prefix = {prefix:?}\n"),
            )
            .unwrap();
            assert!(Config::load(dir.path()).is_ok(), "{prefix:?} should load");
        }
    }

    #[test]
    fn test_malformed_toml_produces_error() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "{{not valid toml").unwrap();

        let result = Config::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_default_toml_package_lists_are_top_level() {
        let content = Config::default_toml();
        let config: Config = toml::from_str(content).unwrap();
        assert!(config.ignore.is_empty());
        assert!(config.private.is_empty());

        let with_ignore = content.replace("ignore = []", r#"ignore = ["foo", "bar"]"#);
        let config: Config = toml::from_str(&with_ignore).unwrap();
        assert_eq!(config.ignore, vec!["foo", "bar"]);

        let with_private = content.replace("private = []", r#"private = ["private-tool"]"#);
        let config: Config = toml::from_str(&with_private).unwrap();
        assert_eq!(config.private, vec!["private-tool"]);
    }

    #[test]
    fn test_partial_config_fills_defaults() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "dependent_bump = \"minor\"\n",
        )
        .unwrap();

        let config = Config::load(dir.path()).unwrap();

        assert_eq!(config.dependent_bump, DependentBump::Minor);
        assert_eq!(config.changelog.format, ChangelogFormat::PerCrate);
        assert!(config.ecosystem.is_none());
        assert!(config.fixed.is_empty());
        assert!(config.linked.is_empty());
        assert!(config.ignore.is_empty());
        assert!(config.private.is_empty());
    }
}
