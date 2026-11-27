//! `brain connect`: registers the installed brain as an AI tool's MCP server, and its hooks as a
//! plugin, through that tool's own commands, so brain never edits another app's settings or
//! instructions.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Context;
use brain::update;
use brain_platform::dirs::Dirs;
use brain_platform::service;

use crate::plugin;

/// The installed brain, which every door runs, so an update reaches every door at once.
pub fn installed(dirs: &Dirs) -> anyhow::Result<PathBuf> {
    let exe = update::install_dir(&dirs.bin).join(update::exe_name());
    anyhow::ensure!(
        exe.is_file(),
        "brain is not installed at {}; {} first",
        exe.display(),
        service::HOW_TO_INSTALL
    );
    Ok(exe)
}

/// `brain connect claude-code`: `claude mcp add` at user scope, after removing any brain added
/// before, so a second connect replaces the first instead of failing on the name.
pub fn claude_code(dirs: &Dirs, name: Option<&str>) -> anyhow::Result<()> {
    if let Some(n) = name {
        crate::room::check_name(n)?;
    }
    let exe = installed(dirs)?;
    let _ = Command::new("claude")
        .args(["mcp", "remove", "--scope", "user", "brain"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let status = Command::new("claude")
        .args(["mcp", "add", "--scope", "user", "brain", "--"])
        .arg(&exe)
        .args(["mcp", "--door", "claude-code"])
        // One word, so a name starting with `-` is never read as a flag.
        .args(name.map(|n| format!("--as={n}")))
        .status()
        .context("running claude; is Claude Code installed and on your PATH?")?;
    // The remove above has already run, so a brain connected before is gone too.
    anyhow::ensure!(
        status.success(),
        "claude mcp add failed ({status}); brain is not connected to Claude Code now, so run brain connect claude-code again"
    );
    hooks(dirs, &exe).context(
        "Claude Code can search your brain, but the hooks are not connected; run brain connect claude-code again",
    )?;
    println!("Claude Code can now search your brain; start a new session.");
    Ok(())
}

/// The plugin, through Claude Code's own `plugin` command, replacing one added before.
fn hooks(dirs: &Dirs, exe: &Path) -> anyhow::Result<()> {
    plugin::quotable(exe)?;
    // An installed brain from before `hook` exits 2 on it, which blocks every prompt.
    let knows = Command::new(exe)
        .args(["hook", "session-start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("running {}", exe.display()))?;
    anyhow::ensure!(
        knows.success(),
        "the installed brain at {} has no hooks yet; run brain update first",
        exe.display()
    );
    let folder = plugin::write(dirs, exe)?;
    let _ = Command::new("claude")
        .args(["plugin", "marketplace", "remove", "brain"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let add = ["plugin", "marketplace", "add"];
    claude(Command::new("claude").args(add).arg(folder))?;
    claude(Command::new("claude").args(["plugin", "install", "brain@brain"]))
}

fn claude(command: &mut Command) -> anyhow::Result<()> {
    let status = command.status().context("running claude")?;
    anyhow::ensure!(status.success(), "claude plugin failed ({status})");
    Ok(())
}
