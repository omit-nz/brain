//! Apple code signatures, which a Mac checks before it runs code.

use std::path::Path;

/// Whether this platform checks a release's code signatures before an update installs it: macOS
/// does; no other platform yet.
pub const CHECKS_RELEASES: bool = cfg!(target_os = "macos");

/// The code requirement for a Developer ID Application signature of `team`, in Apple's own form
/// (Apple's Developer ID intermediate and the Developer ID leaf marker), so the team's development
/// and distribution certificates do not pass.
pub fn developer_id(team: &str) -> String {
    format!(
        r#"anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "{team}""#
    )
}

/// Whether `file` is signed as `requirement` asks (`codesign --verify --strict -R`); the error is
/// codesign's own reason.
pub fn check(file: &Path, requirement: &str) -> Result<(), String> {
    let out = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", &format!("-R={requirement}")])
        .arg(file)
        .output()
        .map_err(|e| format!("codesign could not run: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
