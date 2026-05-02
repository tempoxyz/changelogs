use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(
        "could not detect workspace. specify ecosystem with: changelogs --ecosystem <rust|python|go> init"
    )]
    NotInWorkspace,

    #[error("changelogs already initialized")]
    AlreadyInitialized,

    #[error("changelogs not initialized - run `changelogs init` first")]
    NotInitialized,

    #[error("invalid bump type: {0}")]
    InvalidBumpType(String),

    #[error("invalid ecosystem: {0}")]
    InvalidEcosystem(String),

    #[error("package not found: {0}")]
    PackageNotFound(String),

    #[error("failed to parse changelog {0}: {1}")]
    ChangelogParse(String, String),

    #[error("failed to parse config: {0}")]
    ConfigParse(String),

    #[error("no packages selected")]
    NoPackagesSelected,

    #[error("version not found in {0}")]
    VersionNotFound(String),

    #[error("failed to parse version: {0}")]
    VersionParse(String),

    #[error("Python project not found: {0}")]
    PythonProjectNotFound(String),

    #[error("Python dynamic version: {0}")]
    PythonDynamicVersion(String),

    #[error("publish failed: {0}")]
    PublishFailed(String),

    #[error("failed to check PyPI: {0}")]
    PypiCheckFailed(String),

    #[error("Go module not found: {0}")]
    GoModuleNotFound(String),

    #[error("failed to check Go module proxy: {0}")]
    GoProxyCheckFailed(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("toml edit error: {0}")]
    TomlEdit(#[from] toml_edit::TomlError),

    #[error("yaml parse error: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("cargo metadata error: {0}")]
    CargoMetadata(#[from] cargo_metadata::Error),

    #[error("semver parse error: {0}")]
    SemverParse(#[from] semver::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
