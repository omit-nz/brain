//! `brain connect`: registers the installed brain as an AI tool's MCP server through that tool's
//! own command, so brain never edits another app's settings or instructions.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Context;
use brain::update;
use brain_platform::dirs::Dirs;

/// The installed brain, which every door runs, so an update reaches every door at once.
pub fn installed(dirs: &Dirs) -> anyhow::Result<PathBuf> {
    let exe = update::install_dir(&dirs.bin).join(update::exe_name());
    anyhow::ensure!(
        exe.is_file(),
        "brain is not installed at {}; install it with scripts/install.sh first",
        exe.display()
    );
    Ok(exe)
}

/// `brain connect claude-code`: `claude mcp add` at user scope, after removing any brain added
/// before, so a second connect replaces the first instead of failing on the name.
pub fn claude_code(dirs: &Dirs) -> anyhow::Result<()> {
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
        .status()
        .context("running claude; is Claude Code installed and on your PATH?")?;
    // The remove above has already run, so a brain connected before is gone too.
    anyhow::ensure!(
        status.success(),
        "claude mcp add failed ({status}); brain is not connected to Claude Code now, so run brain connect claude-code again"
    );
    println!("Claude Code can now search your brain; start a new session.");
    Ok(())
}
