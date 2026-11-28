//! The name this computer already holds for the person using it.

use std::process::{Command, Stdio};

/// `git config --global user.name`, else the account's full name.
pub fn name() -> Option<String> {
    first_name([git(), full_name()])
}

/// The first of `candidates` that is not blank, trimmed.
fn first_name(candidates: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .map(|name| name.trim().to_string())
        .find(|name| !name.is_empty())
}

/// What `program` prints, when it runs and succeeds.
fn said(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8(out.stdout).ok())
        .flatten()
}

fn git() -> Option<String> {
    // A Mac without the developer tools answers `git` with their installer.
    if cfg!(target_os = "macos") {
        said("/usr/bin/xcode-select", &["-p"])?;
    }
    said("git", &["config", "--global", "user.name"])
}

fn full_name() -> Option<String> {
    if cfg!(target_os = "macos") {
        return said("id", &["-F"]);
    }
    if cfg!(unix) {
        let user = std::env::var("USER").ok()?;
        let entry = said("getent", &["passwd", &user])?;
        return Some(entry.split(':').nth(4)?.split(',').next()?.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::first_name;

    #[test]
    fn a_blank_name_is_passed_over() {
        let found = first_name([
            Some(String::new()),
            Some("  \n".into()),
            Some(" Test Person\n".into()),
        ]);
        assert_eq!(found, Some("Test Person".into()));
        assert_eq!(first_name([None]), None);
    }
}
