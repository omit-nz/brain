//! `brain setup`: what it decides, in what order it writes, and what it leaves alone.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use super::Home;

/// `brain setup` as the tests run it: git knows the person as Test Person and nothing of the
/// machine's own settings.
fn command(h: &Home, args: &[&str]) -> std::process::Command {
    let gitconfig = h.user.path().join(".gitconfig");
    fs::write(&gitconfig, "[user]\nname = Test Person\n").unwrap();
    let mut c = h.command();
    c.env("GIT_CONFIG_GLOBAL", gitconfig)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("setup")
        .args(args);
    c
}

pub(crate) fn setup(h: &Home, args: &[&str], stdin: &str) -> Output {
    let mut child = command(h, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A run that asks nothing may be gone before it is written to.
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    child.wait_with_output().unwrap()
}

fn ran(h: &Home, args: &[&str], stdin: &str) -> String {
    let out = setup(h, args, stdin);
    assert!(
        out.status.success(),
        "setup {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn refused(h: &Home, args: &[&str], says: &str) {
    let out = setup(h, args, "");
    assert_eq!(out.status.code(), Some(1), "setup {args:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(says), "setup {args:?}: {err}");
}

fn config(h: &Home) -> toml::Table {
    fs::read_to_string(h.home.path().join("config/config.toml"))
        .unwrap()
        .parse()
        .unwrap()
}

fn vault_of(h: &Home) -> PathBuf {
    PathBuf::from(config(h)["vault"].as_str().unwrap())
}

fn follows(h: &Home) -> Option<PathBuf> {
    let follow = config(h)
        .get("memory")?
        .get("follow")?
        .as_str()?
        .to_string();
    Some(PathBuf::from(follow))
}

fn set_up(h: &Home) -> bool {
    h.home.path().join("config/config.toml").exists()
}

/// `~/.claude/projects` in the test's home folder, holding one memory.
fn projects(h: &Home) -> PathBuf {
    let projects = h.user.path().join(".claude/projects");
    fs::create_dir_all(projects.join("p/memory")).unwrap();
    fs::write(projects.join("p/memory/a.md"), "a memory\n").unwrap();
    projects.canonicalize().unwrap()
}

fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

#[test]
fn auto_setup_makes_the_folder_the_index_and_the_config() {
    let h = Home::new();
    projects(&h);
    let out = h
        .command()
        .env("GIT_CONFIG_GLOBAL", {
            let path = h.user.path().join(".gitconfig");
            fs::write(&path, "[user]\nname = Test Person\n").unwrap();
            path
        })
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["--now", "2026-11-29T12:00:00Z", "setup", "--auto"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let said = String::from_utf8_lossy(&out.stdout);
    let vault = h.user.path().join("brain");
    for folder in ["0-inbox", "private", "working-memory", "_archive"] {
        assert!(vault.join(folder).is_dir(), "{folder}");
    }
    assert!(h.home.path().join("data/index.db").is_file());
    let config = config(&h);
    assert_eq!(config["me"].as_str(), Some("Test Person"));
    assert_eq!(vault_of(&h), real(&vault));
    assert_eq!(config["setup"]["ran"].to_string(), "\"2026-11-29\"");
    assert!(!config.contains_key("memory") && !config.contains_key("usage"));
    for line in ["not followed", "Usage counts to omit: on", "brain daemon"] {
        assert!(said.contains(line), "{line}: {said}");
    }
}

#[test]
fn auto_setup_follows_nothing_unasked() {
    for args in [&["--auto"][..], &[]] {
        let h = Home::new();
        projects(&h);
        ran(&h, args, "");
        assert_eq!(follows(&h), None, "{args:?}");
    }
}

#[test]
fn a_setup_after_unfollow_stays_off() {
    let h = Home::new();
    h.init_without_setup();
    let projects = projects(&h);
    h.run(&["memory", "follow", projects.to_str().unwrap()]);
    h.run(&["memory", "unfollow"]);
    ran(&h, &["--auto"], "");
    assert_eq!(follows(&h), None);
}

#[test]
fn memory_is_followed_when_asked() {
    for (args, stdin) in [(&["--memory", "on"][..], ""), (&["--guided"], "\n\non\n\n")] {
        let h = Home::new();
        let projects = projects(&h);
        let said = ran(&h, args, stdin);
        assert_eq!(follows(&h), Some(projects), "{args:?}");
        assert!(said.contains("followed from"), "{said}");
    }
}

#[test]
fn memory_on_keeps_the_folder_already_followed() {
    let h = Home::new();
    h.init_without_setup();
    projects(&h);
    let other = tempfile::tempdir().unwrap();
    h.run(&["memory", "follow", other.path().to_str().unwrap()]);
    ran(&h, &["--memory", "on"], "");
    assert_eq!(follows(&h), Some(real(other.path())));
}

#[test]
fn memory_on_with_no_projects_folder_says_so() {
    let h = Home::new();
    let said = ran(&h, &["--memory", "on"], "");
    assert!(said.contains("Auto-memory: no folder at"), "{said}");
    assert_eq!(follows(&h), None);
}

#[test]
fn memory_off_stops_following() {
    let h = Home::new();
    h.init_without_setup();
    let projects = projects(&h);
    h.run(&["memory", "follow", projects.to_str().unwrap()]);
    ran(&h, &["--memory", "off"], "");
    assert_eq!(follows(&h), None);
}

#[test]
fn a_second_setup_keeps_what_the_person_switched_off() {
    let h = Home::new();
    ran(&h, &["--me", "Ann Example"], "");
    let vault = vault_of(&h);
    h.run(&["usage", "off"]);
    ran(&h, &["--auto"], "");
    let config = config(&h);
    assert_eq!(config["usage"]["send"].as_bool(), Some(false));
    assert_eq!(config["me"].as_str(), Some("Ann Example"));
    assert_eq!(vault_of(&h), vault);
}

#[test]
fn a_flag_answers_its_question() {
    let h = Home::new();
    projects(&h);
    let vault = h.user.path().join("mine");
    let said = ran(
        &h,
        &[
            "--guided",
            "--me",
            "Ann Example",
            "--vault",
            vault.to_str().unwrap(),
            "--memory",
            "off",
            "--usage",
            "off",
        ],
        "",
    );
    for question in ["Your name", "Folder for your notes", "(on/off)"] {
        assert!(!said.contains(question), "{question}: {said}");
    }
    assert_eq!(vault_of(&h), real(&vault));
    assert_eq!(config(&h)["usage"]["send"].as_bool(), Some(false));
}

#[test]
fn guided_setup_asks_and_an_empty_answer_keeps_the_default() {
    let h = Home::new();
    projects(&h);
    let said = ran(&h, &["--guided"], "Ann Example\n\nmaybe\non\n\n");
    for question in [
        "Your name",
        "Folder for your notes",
        "Keep copies of Claude Code's auto-memory? (on/off)",
        "Send the day's usage counts to omit? (on/off)",
    ] {
        assert!(said.contains(question), "{question}: {said}");
    }
    assert_eq!(config(&h)["me"].as_str(), Some("Ann Example"));
    assert_eq!(vault_of(&h), real(&h.user.path().join("brain")));
    assert!(follows(&h).is_some());
    assert!(!config(&h).contains_key("usage"));
}

#[test]
fn a_typed_home_path_lands_under_the_home() {
    let h = Home::new();
    ran(&h, &["--guided"], "\n~/notes\n\n\n");
    assert_eq!(vault_of(&h), real(&h.user.path().join("notes")));
    assert!(!h.vault.path().join("~").exists());
}

#[test]
fn a_relative_vault_is_refused_and_makes_nothing() {
    let h = Home::new();
    refused(&h, &["--vault", "notes"], "full path");
    assert!(!h.vault.path().join("notes").exists());
    assert!(!set_up(&h));
}

#[test]
fn setup_does_not_switch_a_folder_that_holds_memory_copies() {
    let h = Home::new();
    h.init_without_setup();
    let projects = projects(&h);
    h.run(&["memory", "follow", projects.to_str().unwrap()]);
    h.run(&["memory", "run"]);
    let before = vault_of(&h);
    let other = tempfile::tempdir().unwrap();
    refused(
        &h,
        &["--vault", other.path().to_str().unwrap()],
        "stop following",
    );
    assert_eq!(vault_of(&h), before);
}

#[test]
fn setup_refuses_a_home_folder_as_the_vault() {
    let h = Home::new();
    refused(
        &h,
        &["--vault", h.user.path().to_str().unwrap()],
        "your home folder",
    );
    assert!(!set_up(&h));
}

#[test]
fn a_missing_configured_vault_is_not_made_again() {
    let h = Home::new();
    h.init_without_setup();
    let gone = vault_of(&h).join("gone");
    let path = h.home.path().join("config/config.toml");
    let mut config = config(&h);
    config.insert("vault".into(), gone.to_str().unwrap().into());
    fs::write(&path, config.to_string()).unwrap();
    refused(&h, &["--auto"], "is not there");
    assert!(!gone.exists());
}

#[test]
fn setup_takes_brain_notes_when_brain_holds_something_else() {
    let h = Home::new();
    fs::create_dir_all(h.user.path().join("brain")).unwrap();
    fs::write(h.user.path().join("brain/main.rs"), "fn main() {}\n").unwrap();
    ran(&h, &["--auto"], "");
    assert_eq!(vault_of(&h), real(&h.user.path().join("brain-notes")));
    assert_eq!(
        fs::read_dir(h.user.path().join("brain")).unwrap().count(),
        1
    );
}

#[test]
fn setup_with_both_default_folders_taken_writes_nothing() {
    let h = Home::new();
    for name in ["brain", "brain-notes"] {
        fs::create_dir_all(h.user.path().join(name).join("src")).unwrap();
    }
    refused(&h, &["--auto"], "--vault");
    assert!(!set_up(&h));
}

#[test]
fn setup_leaves_an_unreadable_config_as_it_is() {
    let h = Home::new();
    fs::create_dir_all(h.home.path().join("config")).unwrap();
    let path = h.home.path().join("config/config.toml");
    fs::write(&path, "[[[").unwrap();
    refused(&h, &["--auto"], "config.toml");
    assert_eq!(fs::read_to_string(&path).unwrap(), "[[[");
    assert!(!h.user.path().join("brain").exists());
}

#[cfg(unix)]
#[test]
fn setup_makes_its_data_folder_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let h = Home::new();
    ran(&h, &["--auto"], "");
    let mode = fs::metadata(h.home.path().join("data"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn the_config_is_written_last() {
    let h = Home::new();
    fs::create_dir_all(h.home.path().join("data/index.db")).unwrap();
    assert_eq!(setup(&h, &["--auto"], "").status.code(), Some(1));
    assert!(!set_up(&h));
}

#[test]
fn setup_waits_for_the_setup_lock() {
    let h = Home::new();
    let data = h.home.path().join("data");
    brain_platform::fs::create_dir_private(&data).unwrap();
    let held = brain_platform::fs::Lock::acquire(&data.join("setup.lock")).unwrap();
    let mut child = command(&h, &["--auto"])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert!(child.try_wait().unwrap().is_none(), "it did not wait");
    assert!(!set_up(&h));
    drop(held);
    assert!(child.wait().unwrap().success());
}

#[test]
fn setup_switching_usage_off_holds_today_back() {
    use brain_index::activity::Activity;
    use brain_platform::clock::{Clock, SystemClock};
    let h = Home::new();
    ran(&h, &["--auto"], "");
    h.run(&["add", "a note"]);
    ran(&h, &["--usage", "off"], "");
    let activity = Activity::open(&h.home.path().join("data/activity.db")).unwrap();
    let today = SystemClock.today();
    assert!(activity.send_from().unwrap().unwrap() > today);
}

#[test]
fn setup_moves_an_unreadable_prompt_state_aside() {
    let h = Home::new();
    let data = h.home.path().join("data");
    brain_platform::fs::create_dir_private(&data).unwrap();
    fs::write(data.join("setup.json"), "{not json").unwrap();
    ran(&h, &["--auto"], "");
    assert!(!data.join("setup.json").exists());
    let aside = fs::read_dir(&data)
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            name.to_string_lossy().starts_with("setup-unreadable-")
        })
        .count();
    assert_eq!(aside, 1);
}
