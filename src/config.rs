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
    pub ai: AiConfig,
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
            ai: AiConfig::default(),
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

        Ok(config)
    }

    pub fn save(&self, changelog_dir: &Path) -> Result<()> {
        let config_path = changelog_dir.join("config.toml");
        let content =
            toml::to_string_pretty(self).map_err(|e| Error::ConfigParse(e.to_string()))?;
        std::fs::write(config_path, content)?;
        Ok(())
    }

    pub fn default_toml() -> &'static str {
        r#"# Ecosystem: "rust" | "python" (auto-detected if not specified)
# ecosystem = "rust"

# How to bump packages that depend on changed packages
# "patch" | "minor" | "none"
dependent_bump = "patch"

# Packages to ignore
ignore = []

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
            ai: AiConfig {
                command: Some("test-cmd".into()),
            },
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
        assert_eq!(loaded.ai.command.as_deref(), Some("test-cmd"));
    }

    #[test]
    fn test_malformed_toml_produces_error() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("config.toml"), "{{not valid toml").unwrap();

        let result = Config::load(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_default_toml_ignore_is_top_level() {
        let content = Config::default_toml();
        let config: Config = toml::from_str(content).unwrap();
        // ignore must parse as a top-level field, not silently land under [changelog]
        assert!(config.ignore.is_empty());
        // Verify it's actually parsed by setting a value
        let with_ignore = content.replace("ignore = []", r#"ignore = ["foo", "bar"]"#);
        let config: Config = toml::from_str(&with_ignore).unwrap();
        assert_eq!(config.ignore, vec!["foo", "bar"]);
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
    }
}
