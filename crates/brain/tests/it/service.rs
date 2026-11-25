//! `brain start`, `brain stop` and the update's restart, against a `launchctl` stub that records
//! its arguments, so no test reaches the real launchd.

use std::fs;
use std::path::{Path, PathBuf};

use super::Home;
use super::update::{archive, key, public};

/// A `launchctl` stub in its own folder: appends each call's arguments as one line to `calls`,
/// and answers `print` as loaded.
struct Stub {
    dir: tempfile::TempDir,
}

impl Stub {
    fn new() -> Stub {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("launchctl");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\nexit 0\n",
                dir.path().join("calls").display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Stub { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("launchctl")
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// A brain home with a fake HOME, an install in `bin/brain-current`, a vault and an index.
struct Setup {
    h: Home,
    user: tempfile::TempDir,
    stub: Stub,
}

impl Setup {
    fn new(install: bool) -> Setup {
        let s = Setup {
            h: Home::new(),
            user: tempfile::tempdir().unwrap(),
            stub: Stub::new(),
        };
        if install {
            let current = s.h.home.path().join("data/bin/brain-current");
            fs::create_dir_all(current.join("lib")).unwrap();
            fs::write(current.join(brain::update::exe_name()), "binary old").unwrap();
        }
        s
    }

    fn brain(&self, args: &[&str]) -> std::process::Output {
        self.h
            .brain()
            .env("HOME", self.user.path())
            .env("BRAIN_LAUNCHCTL", self.stub.path())
            .args(args)
            .output()
            .unwrap()
    }
}

#[cfg(target_os = "macos")]
fn uid() -> String {
    String::from_utf8(
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string()
}

#[cfg(target_os = "macos")]
#[test]
fn start_writes_the_agent_and_bootstraps_it() {
    let s = Setup::new(true);
    s.h.init();
    let out = s.brain(&["start"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let uid = uid();
    let agent = s
        .user
        .path()
        .join("Library/LaunchAgents/nz.omit.brain.plist");
    assert_eq!(
        s.stub.calls(),
        vec![
            format!("enable gui/{uid}/nz.omit.brain"),
            format!("bootout gui/{uid}/nz.omit.brain"),
            format!("bootstrap gui/{uid} {}", agent.display()),
        ]
    );
    assert!(s.user.path().join("Library/Logs/brain").is_dir());
    let plist = fs::read_to_string(&agent).unwrap();
    let exe = s.h.home.path().join("data/bin/brain-current/brain");
    assert!(
        plist.contains(&format!("<string>{}</string>", exe.display())),
        "{plist}"
    );
    assert!(plist.contains(&format!("<string>{}</string>", s.h.home.path().display())));
}

#[cfg(target_os = "macos")]
#[test]
fn stop_boots_out_and_disables() {
    let s = Setup::new(true);
    s.h.init();
    let out = s.brain(&["stop"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let uid = uid();
    assert_eq!(
        s.stub.calls(),
        vec![
            format!("bootout gui/{uid}/nz.omit.brain"),
            format!("disable gui/{uid}/nz.omit.brain"),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn start_refuses_without_an_install() {
    let s = Setup::new(false);
    s.h.init();
    let out = s.brain(&["start"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("scripts/install.sh"));
    assert!(s.stub.calls().is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn start_refuses_without_a_config_or_index() {
    let s = Setup::new(true);
    let out = s.brain(&["start"]);
    assert_eq!(out.status.code(), Some(1), "no config");
    s.h.init();
    let index = s.h.home.path().join("data/index.db");
    fs::create_dir_all(&index).unwrap();
    let out = s.brain(&["start"]);
    assert_eq!(out.status.code(), Some(1), "an index that will not open");
    assert!(s.stub.calls().is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn the_plist_is_not_written_through_a_link() {
    let s = Setup::new(true);
    s.h.init();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::create_dir_all(s.user.path().join("Library")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), s.user.path().join("Library/LaunchAgents"))
        .unwrap();
    let out = s.brain(&["start"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    assert!(s.stub.calls().is_empty());
}

/// A newer release, signed with a test key the debug build is told to trust.
fn newer_release(dir: &Path) -> String {
    let k = key(7);
    archive(dir, "9.9.9", std::slice::from_ref(&k));
    public(&k).iter().map(|b| format!("{b:02x}")).collect()
}

fn update_from(s: &Setup, releases: &Path, hex: &str) -> std::process::Output {
    s.h.brain()
        .env("HOME", s.user.path())
        .env("BRAIN_LAUNCHCTL", s.stub.path())
        .env("BRAIN_TEST_RELEASE_KEY", hex)
        .args(["update", "--from-dir", releases.to_str().unwrap()])
        .output()
        .unwrap()
}

#[cfg(target_os = "macos")]
#[test]
fn update_restarts_a_loaded_daemon() {
    let s = Setup::new(true);
    let releases = tempfile::tempdir().unwrap();
    let hex = newer_release(releases.path());
    let out = update_from(&s, releases.path(), &hex);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("9.9.9 installed"), "{stdout}");
    assert!(stdout.contains("restarted the daemon"), "{stdout}");
    let uid = uid();
    assert_eq!(
        s.stub.calls(),
        vec![
            format!("print gui/{uid}/nz.omit.brain"),
            format!("kickstart -k gui/{uid}/nz.omit.brain"),
        ]
    );
    assert_eq!(
        fs::read_to_string(
            s.h.home
                .path()
                .join("data/bin/brain-current")
                .join(brain::update::exe_name())
        )
        .unwrap(),
        "binary 9.9.9"
    );
}

#[test]
fn an_unsigned_local_release_is_refused() {
    let s = Setup::new(true);
    let releases = tempfile::tempdir().unwrap();
    let hex = newer_release(releases.path());
    let name = format!(
        "brain-9.9.9-{}.tar.gz",
        brain::update::host_target().unwrap()
    );
    let unsigned = releases.path().join(format!("unsigned-{name}"));
    fs::rename(&unsigned, releases.path().join(&name)).unwrap();
    let out = update_from(&s, releases.path(), &hex);
    assert_eq!(out.status.code(), Some(1));
    assert!(s.stub.calls().is_empty());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn start_outside_macos_says_so() {
    let s = Setup::new(true);
    s.h.init();
    let out = s.brain(&["start"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("macOS only"));
}

#[cfg(not(target_os = "macos"))]
#[test]
fn an_update_outside_macos_restarts_nothing_and_succeeds() {
    let s = Setup::new(true);
    let releases = tempfile::tempdir().unwrap();
    let hex = newer_release(releases.path());
    let out = update_from(&s, releases.path(), &hex);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("9.9.9 installed"), "{stdout}");
    assert!(!stdout.contains("restarted"), "{stdout}");
}
