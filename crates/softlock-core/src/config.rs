//! The settings document the controller stores. The wire format stays TOML.
//! The full schema belongs to a later change; this type keeps the version typed.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    schema_version: u64,
    document: String,
}

impl Config {
    pub fn initial() -> Self {
        Self {
            schema_version: 1,
            document: "schema_version = 1\n".to_string(),
        }
    }

    pub fn schema_version(&self) -> u64 {
        self.schema_version
    }

    pub fn document(&self) -> &str {
        &self.document
    }

    pub fn parse(toml: &str) -> Result<Self, Vec<String>> {
        if toml.trim().is_empty() {
            return Err(vec!["empty config".to_string()]);
        }
        Ok(Self {
            schema_version: schema_version_of(toml).unwrap_or(1),
            document: toml.to_string(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigStore {
    config: Config,
    revision: u64,
}

pub enum ConfigError {
    Invalid(Vec<String>),
    Conflict,
}

impl ConfigStore {
    pub fn new() -> Self {
        Self {
            config: Config::initial(),
            revision: 1,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn document(&self) -> &str {
        self.config.document()
    }

    pub fn apply(&mut self, toml: String, base_revision: u64) -> Result<(), ConfigError> {
        if base_revision != self.revision {
            return Err(ConfigError::Conflict);
        }
        match Config::parse(&toml) {
            Ok(config) => {
                self.config = config;
                self.revision = self.revision.saturating_add(1);
                Ok(())
            }
            Err(errors) => Err(ConfigError::Invalid(errors)),
        }
    }
}

fn schema_version_of(toml: &str) -> Option<u64> {
    for line in toml.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("schema_version") else {
            continue;
        };
        let Some(rest) = rest.trim().strip_prefix('=') else {
            continue;
        };
        return rest.trim().parse().ok();
    }
    None
}
