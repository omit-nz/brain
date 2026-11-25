//! The daemon as a launchd agent: macOS only in Phase 1. Elsewhere starting and stopping are
//! unsupported, and an update finds no agent to restart.

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::fs::{Privacy, create_dir_private, write_atomic};

pub const LABEL: &str = "nz.omit.brain";

/// Whether this system runs the daemon as a service.
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// What a "not installed" message tells a person to do: the signed package on macOS, the install
/// script where there is no package yet.
pub const HOW_TO_INSTALL: &str = if cfg!(target_os = "macos") {
    "install brain from brain.pkg"
} else {
    "install brain with scripts/install.sh"
};

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The agent's property list: runs `exe daemon` at login and again after a crash, never after a
/// clean exit, with its output in `log`.
pub fn plist(exe: &Path, log: &Path, brain_home: Option<&Path>) -> String {
    let s = |p: &Path| xml(&p.to_string_lossy());
    let env = brain_home.map_or(String::new(), |h| {
        format!(
            "  <key>EnvironmentVariables</key>\n  <dict>\n    <key>BRAIN_HOME</key>\n    <string>{}</string>\n  </dict>\n",
            s(h)
        )
    });
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>StandardOutPath</key>
  <string>{log}</string>
{env}</dict>
</plist>
"#,
        exe = s(exe),
        log = s(log),
    )
}

/// `launchctl`, named by the path the caller passes, so a test can hand it a stub.
pub struct Launchctl(pub PathBuf);

impl Launchctl {
    /// Writes the agent under `home` and loads it, replacing any agent loaded before, and lets
    /// it start at every login again.
    pub fn start(&self, home: &Path, plist: &str) -> io::Result<()> {
        unsupported()?;
        create_dir_private(&home.join("Library/Logs/brain"))?;
        let rel = Path::new("Library/LaunchAgents").join(format!("{LABEL}.plist"));
        write_atomic(home, &rel, plist.as_bytes(), Privacy::Normal)?;
        let domain = domain()?;
        let service = format!("{domain}/{LABEL}");
        self.run(&["enable", &service])?;
        let _ = self.run(&["bootout", &service]);
        self.run(&["bootstrap", &domain, &home.join(&rel).to_string_lossy()])
    }

    /// Unloads the agent and keeps it from starting at the next login.
    pub fn stop(&self) -> io::Result<()> {
        unsupported()?;
        let service = format!("{}/{LABEL}", domain()?);
        self.run(&["bootout", &service])?;
        self.run(&["disable", &service])
    }

    /// Restarts the agent when one is loaded; `Ok(false)` when none is, or on a system without
    /// agents.
    pub fn restart_if_loaded(&self) -> io::Result<bool> {
        if !SUPPORTED {
            return Ok(false);
        }
        let service = format!("{}/{LABEL}", domain()?);
        if self.run(&["print", &service]).is_err() {
            return Ok(false);
        }
        self.run(&["kickstart", "-k", &service])?;
        Ok(true)
    }

    fn run(&self, args: &[&str]) -> io::Result<()> {
        let out = Command::new(&self.0).args(args).output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "launchctl {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }
}

fn unsupported() -> io::Result<()> {
    if SUPPORTED {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::Unsupported,
            "the daemon runs as a service on macOS only in this release",
        ))
    }
}

#[cfg(target_os = "macos")]
fn domain() -> io::Result<String> {
    Ok(format!("gui/{}", rustix::process::getuid().as_raw()))
}

#[cfg(not(target_os = "macos"))]
fn domain() -> io::Result<String> {
    unsupported().map(|()| String::new())
}
