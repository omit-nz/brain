use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, write_atomic};
use serde::{Deserialize, Serialize};

/// Days from a note's creation to its first review.
pub const REVIEW_DAYS: i64 = 90;

pub(crate) const FILE: &str = "config.toml";

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub vault: PathBuf,
    pub me: String,
    #[serde(default, skip_serializing_if = "UsageConfig::is_default")]
    pub usage: UsageConfig,
    #[serde(default, skip_serializing_if = "SessionConfig::is_default")]
    pub session: SessionConfig,
    #[serde(default, skip_serializing_if = "MemoryConfig::is_default")]
    pub memory: MemoryConfig,
}

/// The Claude Code auto-memory brain keeps its copies in step with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Claude Code's projects folder.
    pub follow: Option<PathBuf>,
}

impl MemoryConfig {
    fn is_default(&self) -> bool {
        *self == MemoryConfig::default()
    }
}

/// What a Claude Code session starts with.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Memory folders whose `MEMORY.md` is shown.
    pub indexes: Vec<PathBuf>,
}

impl SessionConfig {
    fn is_default(&self) -> bool {
        *self == SessionConfig::default()
    }
}

/// Whether the day's usage counts go to omit, and where. On unless the owner switches it off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageConfig {
    pub send: bool,
    pub endpoint: String,
}

impl Default for UsageConfig {
    fn default() -> UsageConfig {
        UsageConfig {
            send: true,
            endpoint: "https://usage.brain.omit.nz/v1/day".into(),
        }
    }
}

impl UsageConfig {
    fn is_default(&self) -> bool {
        *self == UsageConfig::default()
    }
}

impl Config {
    pub fn load(dirs: &Dirs) -> anyhow::Result<Config> {
        let path = dirs.config.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => bail!(
                "brain has no vault yet; in your notes folder, run `brain init --me <your name>`"
            ),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn save(&self, dirs: &Dirs) -> anyhow::Result<()> {
        let text = toml::to_string(self)?;
        write_atomic(
            &dirs.config,
            Path::new(FILE),
            text.as_bytes(),
            Privacy::Normal,
        )?;
        Ok(())
    }
}
