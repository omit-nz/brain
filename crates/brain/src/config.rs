use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Lock, Privacy, write_atomic};
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// Days from a note's creation to its first review.
pub const REVIEW_DAYS: i64 = 90;

pub(crate) const FILE: &str = "config.toml";

/// Held by whoever writes the config, from reading it to saving it.
const LOCK: &str = "config.lock";

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
    #[serde(default, skip_serializing_if = "SetupConfig::is_default")]
    pub setup: SetupConfig,
}

/// What `brain setup` leaves behind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SetupConfig {
    /// The day setup last ran to its end.
    pub ran: Option<Date>,
}

impl SetupConfig {
    fn is_default(&self) -> bool {
        *self == SetupConfig::default()
    }
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
    /// Every setting at its default over `vault`, owned by `me`.
    pub fn new(vault: PathBuf, me: String) -> Config {
        Config {
            vault,
            me,
            usage: UsageConfig::default(),
            session: SessionConfig::default(),
            memory: MemoryConfig::default(),
            setup: SetupConfig::default(),
        }
    }

    /// The config, or `None` when there is none yet; one that cannot be read is an error.
    pub fn find(dirs: &Dirs) -> anyhow::Result<Option<Config>> {
        let path = dirs.config.join(FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        toml::from_str(&text)
            .map(Some)
            .with_context(|| format!("reading {}", path.display()))
    }

    pub fn load(dirs: &Dirs) -> anyhow::Result<Config> {
        existing(Config::find(dirs)?)
    }

    /// The one way the config is written: what is there is read, changed and saved under the
    /// lock, so no writer drops what another just saved. Returns what was saved.
    pub fn update(
        dirs: &Dirs,
        change: impl FnOnce(Option<Config>) -> anyhow::Result<Config>,
    ) -> anyhow::Result<Config> {
        let _lock = Lock::acquire(&dirs.config.join(LOCK))
            .with_context(|| format!("locking {}", dirs.config.join(LOCK).display()))?;
        let config = change(Config::find(dirs)?)?;
        config.save(dirs)?;
        Ok(config)
    }

    /// `update` for a change to a config that must already be there: with none, nothing is made.
    pub fn edit(dirs: &Dirs, change: impl FnOnce(&mut Config)) -> anyhow::Result<Config> {
        Config::load(dirs)?;
        Config::update(dirs, |before| {
            let mut config = existing(before)?;
            change(&mut config);
            Ok(config)
        })
    }

    fn save(&self, dirs: &Dirs) -> anyhow::Result<()> {
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

/// `before`, or what to say when there is no config.
fn existing(before: Option<Config>) -> anyhow::Result<Config> {
    match before {
        Some(config) => Ok(config),
        None => {
            bail!("brain has no vault yet; in your notes folder, run `brain init --me <your name>`")
        }
    }
}
