//! Apple code signatures, which a Mac checks before it runs code.

use std::path::Path;

/// The code requirement a release's binary and runtime library must meet before an update installs
/// them: on macOS, a signature rooted in Apple's certificate authority whose leaf names `team`. No
/// other platform checks one yet.
pub fn release_requirement(team: &str) -> Option<String> {
    cfg!(target_os = "macos")
        .then(|| format!(r#"anchor apple generic and certificate leaf[subject.OU] = "{team}""#))
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
