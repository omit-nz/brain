use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, write_atomic};
use serde::{Deserialize, Serialize};

/// Days from a note's creation to its first review.
pub const REVIEW_DAYS: i64 = 90;

const FILE: &str = "config.toml";

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub vault: PathBuf,
    pub me: String,
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
