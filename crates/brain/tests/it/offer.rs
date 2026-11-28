//! The state setup's offer is kept in: `setup.json`, and `brain setup reminders`.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::Home;

fn state_path(h: &Home) -> PathBuf {
    h.home.path().join("data/setup.json")
}

fn state(h: &Home) -> Value {
    serde_json::from_str(&fs::read_to_string(state_path(h)).unwrap()).unwrap()
}

fn written(h: &Home, name: &str, text: &str) {
    let data = h.home.path().join("data");
    brain_platform::fs::create_dir_private(&data).unwrap();
    fs::write(data.join(name), text).unwrap();
}

/// The names of the unreadable files set aside, sorted.
fn set_aside(h: &Home) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(h.home.path().join("data"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("setup-unreadable-"))
        .collect();
    names.sort();
    names
}

#[test]
fn reminders_off_and_on_need_no_config_and_keep_the_rest() {
    let h = Home::new();
    written(&h, "setup.json", r#"{"offered_at":"2026-11-01T00:00:00Z"}"#);
    h.run(&["setup", "reminders", "off"]);
    assert_eq!(state(&h)["reminders_off"], json!(true));
    assert_eq!(state(&h)["offered_at"], json!("2026-11-01T00:00:00Z"));
    assert!(!h.home.path().join("config/config.toml").exists());
    h.run(&["setup", "reminders", "on"]);
    assert_eq!(state(&h)["reminders_off"], json!(false));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(state_path(&h)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn reminders_waits_for_the_setup_lock() {
    let h = Home::new();
    written(&h, "setup.json", "{}");
    let held = brain_platform::fs::Lock::acquire(&h.home.path().join("data/setup.lock")).unwrap();
    let mut child = h
        .command()
        .args(["setup", "reminders", "off"])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert!(child.try_wait().unwrap().is_none(), "it did not wait");
    drop(held);
    assert!(child.wait().unwrap().success());
}

#[test]
fn reminders_over_a_corrupt_file_changes_nothing_else() {
    let h = Home::new();
    written(&h, "setup.json", "{not json");
    let out = h
        .brain()
        .args(["--now", "2026-11-29T00:00:00Z", "setup", "reminders", "off"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("run the command again"), "{err}");
    assert!(!state_path(&h).exists());
    let aside = set_aside(&h);
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert!(
        aside[0].starts_with("setup-unreadable-2026-11-"),
        "{aside:?}"
    );
    assert_eq!(
        fs::read_to_string(h.home.path().join("data").join(&aside[0])).unwrap(),
        "{not json"
    );
}

#[test]
fn status_names_a_set_aside_prompt_state() {
    let name = "setup-unreadable-2026-11-29.json";
    let h = Home::new();
    written(&h, name, "{not json");
    let out = h.brain().arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(name), "{err}");
    h.init();
    let says = |h: &Home| {
        let text = h.run(&["status"]);
        let line = text.lines().find(|l| l.contains(name));
        let line = line.unwrap_or_else(|| panic!("{text}"));
        assert!(line.contains("reminders started again"), "{line}");
        assert!(line.contains("brain setup reminders off"), "{line}");
        assert_eq!(
            h.json(&["status", "--json"])["setup_unreadable"],
            json!([name])
        );
    };
    says(&h);
    // An index built over one vault, asked about from another.
    h.run(&["index"]);
    let other = tempfile::tempdir().unwrap();
    h.run(&[
        "init",
        "--me",
        "robin",
        "--vault",
        other.path().to_str().unwrap(),
    ]);
    says(&h);
}
