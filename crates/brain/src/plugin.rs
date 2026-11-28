//! The Claude Code plugin: a marketplace folder brain writes into its own data folder, holding
//! hooks that each make one `brain hook <point>` call. Claude Code reads the folder where it is,
//! so an update only has to rewrite it.

use std::path::{Path, PathBuf};

use anyhow::Context;
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, write_atomic};

use crate::connect;

const FOLDER: &str = "claude-plugin";

/// Each file's template and where it goes in the folder.
const FILES: [(&str, &str); 4] = [
    (
        ".claude-plugin/marketplace.json",
        include_str!("../../../plugin/marketplace.json"),
    ),
    (
        "brain/.claude-plugin/plugin.json",
        include_str!("../../../plugin/plugin.json"),
    ),
    (
        "brain/hooks/hooks.json",
        include_str!("../../../plugin/hooks.json"),
    ),
    (
        "brain/skills/brain/SKILL.md",
        include_str!("../../../plugin/SKILL.md"),
    ),
];

/// `exe` as a hook command can name it.
pub fn quotable(exe: &Path) -> anyhow::Result<&str> {
    let path = exe.to_str().context("brain's path is not UTF-8")?;
    // The hook command quotes the path for a shell, which a single quote would end.
    anyhow::ensure!(
        !path.contains('\''),
        "{path} holds a single quote, which a hook command cannot carry"
    );
    Ok(path)
}

/// Writes the plugin whose hooks call `exe`, and returns the marketplace folder.
pub fn write(dirs: &Dirs, exe: &Path) -> anyhow::Result<PathBuf> {
    let path = quotable(exe)?;
    let quoted = serde_json::to_string(path)?;
    let inside_json = &quoted[1..quoted.len() - 1];
    for (rel, template) in FILES {
        let text = template
            .replace("{{brain}}", inside_json)
            .replace("{{version}}", env!("CARGO_PKG_VERSION"));
        let rel = Path::new(FOLDER).join(rel);
        write_atomic(&dirs.data, &rel, text.as_bytes(), Privacy::Normal)
            .with_context(|| format!("writing {}", rel.display()))?;
    }
    Ok(dirs.data.join(FOLDER))
}

/// Brings a plugin written before up to this release. Writes nothing where none was written.
pub fn refresh(dirs: &Dirs) -> anyhow::Result<()> {
    if dirs.data.join(FOLDER).is_dir() {
        write(dirs, &connect::installed(dirs)?)?;
    }
    Ok(())
}
