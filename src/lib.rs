pub mod changelog_entry;
pub mod changelog_writer;
pub mod config;
pub mod ecosystems;
pub mod error;
pub mod graph;
pub mod plan;
pub mod workspace;

use serde::{Deserialize, Serialize};

pub use changelog_entry::{Changelog, Release};
pub use config::Config;
pub use ecosystems::{Ecosystem, Package, PublishResult, SkipReason};
pub use plan::{PackageRelease, ReleasePlan};
pub use workspace::Workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BumpType {
    Patch,
    Minor,
    Major,
}

impl std::fmt::Display for BumpType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BumpType::Patch => write!(f, "patch"),
            BumpType::Minor => write!(f, "minor"),
            BumpType::Major => write!(f, "major"),
        }
    }
}

impl std::str::FromStr for BumpType {
    type Err = error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "patch" => Ok(BumpType::Patch),
            "minor" => Ok(BumpType::Minor),
            "major" => Ok(BumpType::Major),
            _ => Err(error::Error::InvalidBumpType(s.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_bump_type_from_str_valid() {
        assert_eq!(BumpType::from_str("patch").unwrap(), BumpType::Patch);
        assert_eq!(BumpType::from_str("minor").unwrap(), BumpType::Minor);
        assert_eq!(BumpType::from_str("major").unwrap(), BumpType::Major);
        assert_eq!(BumpType::from_str("Patch").unwrap(), BumpType::Patch);
        assert_eq!(BumpType::from_str("MAJOR").unwrap(), BumpType::Major);
    }

    #[test]
    fn test_bump_type_from_str_invalid() {
        assert!(BumpType::from_str("invalid").is_err());
        assert!(BumpType::from_str("").is_err());
        assert!(BumpType::from_str("micro").is_err());
    }
}
