//! Clearing the stamped review dates of followed memories, archived ones too. Whatever it meets,
//! follow goes on and every copy stays brain's.

use sha2::{Digest, Sha256};

use super::*;
use crate::digest;

const LEDGER: &str = "data/import-ledger.json";
#[cfg(unix)]
const ARCHIVE: &str = "data/follow-archive.json";
const CLEAR: [&str; 5] = [
    "import",
    "clear-review-dates",
    "--imported-on",
    "2026-11-25",
    "--apply",
];

/// Leaves `name`'s followed copy as 0.2.5 wrote it on 2026-11-25: with a review date 1 to 90
/// days on, chosen by a hash of its id, and recorded so in the ledger.
fn stamp(h: &Home, name: &str) {
    let path = copy_path(h.vault.path(), name);
    let text = fs::read_to_string(&path).unwrap();
    let id = text.lines().find_map(|l| l.strip_prefix("id: ")).unwrap();
    let hashed = Sha256::digest(id.as_bytes());
    let days = u64::from_le_bytes(hashed[..8].try_into().unwrap()) % 90;
    let review = jiff::civil::date(2026, 11, 25)
        .checked_add(jiff::ToSpan::days(days as i64 + 1))
        .unwrap();
    let stamped = text.replacen(
        "owner: robin\n",
        &format!("owner: robin\nreview: {review}\n"),
        1,
    );
    assert_ne!(text, stamped);
    let ledger_at = h.home.path().join(LEDGER);
    let recorded = fs::read_to_string(&ledger_at).unwrap();
    let (old, new) = (digest(text.as_bytes()), digest(stamped.as_bytes()));
    assert!(recorded.contains(&old));
    fs::write(&ledger_at, recorded.replace(&old, &new)).unwrap();
    fs::write(&path, &stamped).unwrap();
}

fn archive_path(h: &Home, name: &str) -> PathBuf {
    h.vault
        .path()
        .join("_archive/working-memory/-proj")
        .join(format!("{name}.md"))
}

/// Three followed memories as 0.2.5 left them, `one` since archived.
fn stamped_with_one_archived() -> (Home, Projects) {
    let (h, p) = following(&["one", "stays", "keeps"]);
    for name in ["one", "stays", "keeps"] {
        stamp(&h, name);
    }
    p.remove("one");
    pass(&h);
    let said = pass(&h);
    assert!(said.contains("1 archived"), "{said}");
    (h, p)
}

/// `one`'s source returns as it was, and a change to it then reaches its copy.
fn comes_back_and_takes_a_change(h: &Home, p: &Projects) {
    p.write("one", "one first");
    let said = pass(h);
    assert!(said.contains("1 restored"), "{said}");
    p.write("one", "one second");
    let said = pass(h);
    assert!(said.contains("1 updated"), "{said}");
    assert!(copy(h, "one").unwrap().contains("one second"));
}

#[test]
fn an_archived_copys_stamped_date_is_cleared_too() {
    let (h, p) = stamped_with_one_archived();
    let said = h.run(&CLEAR);
    assert!(said.contains("3 review dates cleared"), "{said}");
    let archived = fs::read_to_string(archive_path(&h, "one")).unwrap();
    assert!(!archived.contains("review:"), "{archived}");
    comes_back_and_takes_a_change(&h, &p);
    assert!(!copy(&h, "one").unwrap().contains("review:"));
}

/// An archived copy brain may not reach is there all the same, and is named.
#[cfg(unix)]
#[test]
fn an_archived_copy_under_an_unreadable_folder_is_named() {
    use std::os::unix::fs::PermissionsExt;
    let (h, _p) = stamped_with_one_archived();
    let folder = h.vault.path().join("_archive/working-memory/-proj");
    let mode = |m| fs::set_permissions(&folder, fs::Permissions::from_mode(m)).unwrap();
    mode(0o000);
    let said = h.run(&CLEAR);
    mode(0o700);
    assert!(said.contains("2 review dates cleared"), "{said}");
    assert!(
        said.contains("not looked at:\n  _archive/working-memory/-proj/one.md"),
        "{said}"
    );
}

/// A follow pass started while a clearing runs waits for it, and both leave their work.
#[test]
fn a_follow_pass_started_during_a_clearing_waits() {
    let (h, p) = stamped_with_one_archived();
    p.write("stays", "stays second");
    let (clearing, waiting) = h.held(&CLEAR);
    let mut follow = h
        .command()
        .args(["memory", "run"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(follow.try_wait().unwrap().is_none(), "it did not wait");
    fs::remove_file(waiting).unwrap();
    let cleared = clearing.wait_with_output().unwrap();
    assert!(cleared.status.success(), "{cleared:?}");
    let followed = follow.wait_with_output().unwrap();
    let said = String::from_utf8_lossy(&followed.stdout);
    assert!(said.contains("1 updated"), "{said}");
    assert!(said.contains("0 kept as they are"), "{said}");
    assert!(copy(&h, "stays").unwrap().contains("stays second"));
    assert!(!copy(&h, "keeps").unwrap().contains("review:"));
    comes_back_and_takes_a_change(&h, &p);
}

/// The ledger saved, the archive's records not, or both saved and no note written.
#[test]
fn a_clearing_killed_before_writing_loses_no_archived_record() {
    for (hook, at) in [
        ("BRAIN_TEST_ABORT_BETWEEN_CLEAR_SAVES", "1"),
        ("BRAIN_TEST_ABORT_AFTER_CLEARED", "0"),
        ("BRAIN_TEST_ABORT_AFTER_CLEARED", "1"),
    ] {
        let (h, p) = stamped_with_one_archived();
        let out = h.brain().args(CLEAR).env(hook, at).output().unwrap();
        assert!(!out.status.success(), "{hook}");
        // The copies still in their places take their sources' changes too, written or not.
        p.write("stays", "stays second");
        p.write("keeps", "keeps second");
        let said = pass(&h);
        assert!(said.contains("2 updated"), "{hook}: {said}");
        assert!(said.contains("0 kept as they are"), "{hook}: {said}");
        comes_back_and_takes_a_change(&h, &p);
    }
}

/// An archive save that fails stops the run with the ledger ahead, which holds nothing.
#[cfg(target_os = "macos")]
#[test]
fn archive_records_that_cannot_be_saved_stop_the_run_and_hold_nothing() {
    let (h, p) = stamped_with_one_archived();
    let records = h.home.path().join(ARCHIVE);
    let flag = |f: &str| {
        let done = std::process::Command::new("chflags")
            .args([f.as_ref(), records.as_os_str()])
            .status()
            .unwrap();
        assert!(done.success());
    };
    flag("uchg");
    let out = h.brain().args(CLEAR).output().unwrap();
    flag("nouchg");
    assert!(!out.status.success());
    assert!(copy(&h, "stays").unwrap().contains("review:"));
    comes_back_and_takes_a_change(&h, &p);
}

/// The sources either record file holds a copy of.
#[cfg(unix)]
fn records(h: &Home) -> std::collections::BTreeSet<String> {
    let read = |rel: &str| -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(h.home.path().join(rel)).unwrap()).unwrap()
    };
    let archive = read(ARCHIVE);
    let archived = archive.as_array().unwrap().iter();
    read(LEDGER)
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .chain(archived.map(|r| r["key"].as_str().unwrap().to_string()))
        .collect()
}

/// Each of these held follow for good, or lost a record, under an earlier design.
#[cfg(unix)]
#[test]
fn what_it_cannot_look_at_holds_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |at: &Path, m| fs::set_permissions(at, fs::Permissions::from_mode(m)).unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("note.md"), "a note\n").unwrap();
    for case in [
        "archived file deleted",
        "archived file moved back by hand",
        "archived file unreadable",
        "project folder unreadable",
        "project folder swapped for a link",
        "vault unreadable",
    ] {
        let (h, p) = stamped_with_one_archived();
        let archived = archive_path(&h, "one");
        let folder = h.vault.path().join("working-memory/-proj");
        let aside = h.home.path().join("aside");
        match case {
            "archived file deleted" => fs::remove_file(&archived).unwrap(),
            "archived file moved back by hand" => {
                fs::rename(&archived, copy_path(h.vault.path(), "one")).unwrap()
            }
            "archived file unreadable" => mode(&archived, 0o000),
            "project folder unreadable" => mode(&folder, 0o000),
            "vault unreadable" => mode(h.vault.path(), 0o000),
            _ => {
                fs::rename(&folder, &aside).unwrap();
                std::os::unix::fs::symlink(&aside, &folder).unwrap();
            }
        }
        // From outside the vault, which may not be entered.
        let brain = || {
            let mut c = h.brain();
            c.current_dir(h.home.path());
            c
        };
        let out = brain().args(CLEAR).output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.success(),
            case != "vault unreadable",
            "{case}: {out:?}"
        );
        if case != "vault unreadable" && (case.contains("unreadable") || case.contains("link")) {
            assert!(said.contains("not looked at"), "{case}: {said}");
        }
        // Follow and import run while it holds, and drop no record.
        let before = records(&h);
        let ran = brain().args(["memory", "run"]).output().unwrap();
        let import = brain()
            .args(["import", "learning-loop", "--from"])
            .arg(source.path())
            .output()
            .unwrap();
        // Neither can write into a vault it may not read.
        if case != "vault unreadable" {
            assert!(ran.status.success(), "{case}: {ran:?}");
            assert!(import.status.success(), "{case}: {import:?}");
        }
        let lost: Vec<_> = before.difference(&records(&h)).cloned().collect();
        assert!(lost.is_empty(), "{case}: records dropped: {lost:?}");
        match case {
            "archived file unreadable" => mode(&archived, 0o600),
            "project folder unreadable" => mode(&folder, 0o700),
            "vault unreadable" => mode(h.vault.path(), 0o700),
            "project folder swapped for a link" => {
                fs::remove_file(&folder).unwrap();
                fs::rename(&aside, &folder).unwrap();
            }
            _ => {}
        }
        // Follow goes on, and a copy whose source goes is still brain's to archive.
        p.remove("stays");
        pass(&h);
        let said = pass(&h);
        assert!(said.contains("archived: "), "{case}: {said}");
        assert!(
            said.contains("0 kept as they are"),
            "{case}: kept instead of archived: {said}"
        );
    }
}
