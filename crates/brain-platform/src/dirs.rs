use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Where brain keeps its config, data, models and installed binaries. The index lives at
/// `index.db` in `data`.
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub models: PathBuf,
    pub bin: PathBuf,
}

impl Dirs {
    /// Everything under one root, as `$BRAIN_HOME` and every test use it.
    pub fn from_home(home: &Path) -> Dirs {
        let data = home.join("data");
        Dirs {
            config: home.join("config"),
            models: data.join("models"),
            bin: data.join("bin"),
            data,
        }
    }

    pub fn resolve() -> anyhow::Result<Dirs> {
        if let Some(home) = std::env::var_os("BRAIN_HOME") {
            let home = home.to_str().context("BRAIN_HOME is not UTF-8")?;
            return Ok(Dirs::from_home(&parse_brain_home(home)?));
        }
        let p = directories::ProjectDirs::from("nz", "omit", "brain")
            .context("no home directory to keep brain's data in")?;
        let data = p.data_local_dir().to_path_buf();
        Ok(Dirs {
            config: p.config_dir().to_path_buf(),
            models: data.join("models"),
            bin: data.join("bin"),
            data,
        })
    }
}

pub fn parse_brain_home(value: &str) -> anyhow::Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        bail!("BRAIN_HOME must be an absolute path, got {value}");
    }
    Ok(path)
}
