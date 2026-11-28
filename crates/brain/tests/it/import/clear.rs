//! `brain import clear-review-dates`: only the dates an import wrote, and nothing left half done
//! however a run ends.

use std::fs;
use std::process::Output;
use std::time::Duration;

use brain_platform::fs::Lock;

use super::*;
use crate::digest;

/// What an import on `NOW`'s day stamped in 0.2.5: 1 to 90 days out by a hash of the note's id,
/// or of its source's key when the source had none.
const BY_ID: &str = "2027-01-14";
const BY_KEY: &str = "2027-01-16";
const STAMPED_ID: &str = "0195f2e0-0000-7000-8000-000000000001";
const CLEAR: [&str; 4] = [
    "import",
    "clear-review-dates",
    "--imported-on",
    "2026-11-25",
];
const NOTES: [&str; 2] = ["byid", "noid"];

impl Import {
    /// Leaves the copy at `rel` as 0.2.5 wrote it: with `review`, recorded in the ledger.
    fn stamp(&self, rel: &str, review: &str) {
        let before = self.copy(rel);
        let after = before.replacen(
            "owner: robin\n",
            &format!("owner: robin\nreview: {review}\n"),
            1,
        );
        assert_ne!(before, after, "{rel} has no owner line");
        fs::write(self.h.vault.path().join(rel), &after).unwrap();
        let path = self.h.home.path().join("data/import-ledger.json");
        let ledger = fs::read_to_string(&path).unwrap();
        let (old, new) = (digest(before.as_bytes()), digest(after.as_bytes()));
        assert!(ledger.contains(&old), "{rel} is not in the ledger");
        fs::write(path, ledger.replace(&old, &new)).unwrap();
    }

    fn clear(&self, extra: &[&str]) -> String {
        self.h.run(&[&CLEAR[..], extra].concat())
    }

    /// A run with `--apply` that is expected not to succeed, or with a test hook set.
    fn apply_with(&self, hook: Option<(&str, &str)>) -> Output {
        let mut command = self.h.brain();
        command.args(CLEAR).arg("--apply");
        if let Some((name, value)) = hook {
            command.env(name, value);
        }
        command.output().unwrap()
    }

    /// Runs `--apply`, held once its records are saved and before it writes a note while
    /// `meanwhile` runs.
    fn apply_while(&self, meanwhile: impl FnOnce()) -> Output {
        let (running, waiting) = self.h.held(&[&CLEAR[..], &["--apply"]].concat());
        meanwhile();
        fs::remove_file(waiting).unwrap();
        running.wait_with_output().unwrap()
    }

    fn dated(&self) -> usize {
        NOTES
            .iter()
            .filter(|n| line(&self.copy(&format!("3-permanent/{n}.md")), "review").is_some())
            .count()
    }

    /// Changes both stamped sources: brain's own copies are then updated, none a conflict.
    fn both_update(&self) {
        for name in NOTES {
            let path = self.src.path().join(format!("3-permanent/{name}.md"));
            let text = fs::read_to_string(&path).unwrap();
            fs::write(path, format!("{text}more\n")).unwrap();
        }
        let said = self.import(&[]);
        assert!(
            said.contains("2 updated") && said.contains("0 conflicts"),
            "{said}"
        );
    }
}

/// An import with two copies stamped as 0.2.5 stamped them, beside dates that are not its.
fn stamped() -> Import {
    let i = Import::new();
    fs::write(
        i.src.path().join("3-permanent/byid.md"),
        // Its own date sits 90 days before the stamp, as a capture's would. The id is in
        // capitals; the import chose the date by the id as brain writes it.
        format!(
            "---\nid: {}\ndate: 2026-10-16\n---\nby id\n",
            STAMPED_ID.to_uppercase()
        ),
    )
    .unwrap();
    fs::write(i.src.path().join("3-permanent/noid.md"), "no id\n").unwrap();
    i.import(&[]);
    i.stamp("3-permanent/byid.md", BY_ID);
    i.stamp("3-permanent/noid.md", BY_KEY);
    i.h.put(
        "hand.md",
        "---\nid: 0195f2e0-0000-7000-8000-000000000002\nowner: robin\nreview: 2027-06-01\n---\nmine\n",
    );
    // A capture's own date is 90 days on, which can fall where an import's would.
    i.h.put(
        "0-inbox/said.md",
        &format!(
            "---\nid: 0195f2e0-0000-7000-8000-000000000003\ndate: 2026-10-16\nreview: {BY_ID}\n---\nkept\n"
        ),
    );
    i
}

#[test]
fn clearing_previews_without_writing() {
    let i = stamped();
    let before = (
        snapshot(i.h.vault.path()),
        snapshot(&i.h.home.path().join("data")),
    );
    let said = i.clear(&[]);
    assert!(said.contains("2 review dates would be cleared"), "{said}");
    assert!(said.contains("3-permanent/byid.md"), "{said}");
    assert!(said.contains("1 written by capture"), "{said}");
    assert!(said.contains("2 set some other way"), "{said}");
    assert!(said.contains("--apply"), "{said}");
    assert_eq!(snapshot(i.h.vault.path()), before.0);
    let data = snapshot(&i.h.home.path().join("data"));
    assert_eq!(
        data.get("import-ledger.json"),
        before.1.get("import-ledger.json")
    );
}

#[test]
fn clearing_takes_the_line_and_nothing_else() {
    let i = stamped();
    let was = |rel: &str| {
        let at = i.h.vault.path().join(rel);
        (i.copy(rel), fs::metadata(at).unwrap().modified().unwrap())
    };
    let kept: Vec<_> = ["hand.md", "0-inbox/said.md"].map(was).into();
    let (before, changed) = was("3-permanent/byid.md");
    let said = i.clear(&["--apply"]);
    assert!(said.contains("2 review dates cleared"), "{said}");
    assert!(
        said.contains("if this is interrupted, run the same command again"),
        "{said}"
    );
    let (after, still) = was("3-permanent/byid.md");
    assert_eq!(after, before.replace(&format!("review: {BY_ID}\n"), ""));
    assert_eq!(still, changed, "the file's time stays the source's");
    assert_eq!(i.dated(), 0);
    assert_eq!(["hand.md", "0-inbox/said.md"].map(was).to_vec(), kept);
    assert!(i.clear(&["--apply"]).contains("0 review dates cleared"));
    i.both_update();
}

/// An import wrote `review:` and nothing else: another spelling is a person's.
#[test]
fn a_review_key_spelled_another_way_is_left() {
    let i = stamped();
    let rel = "3-permanent/byid.md";
    let theirs = i.copy(rel).replace("review:", "Review:");
    fs::write(i.h.vault.path().join(rel), &theirs).unwrap();
    assert!(i.clear(&["--apply"]).contains("1 review date cleared"));
    assert_eq!(i.copy(rel), theirs);
}

#[test]
fn the_days_that_would_have_written_what_is_left_are_named() {
    let i = stamped();
    // Further from every stamp than any seed reaches.
    let said = i.h.run(&[
        "--now",
        NOW,
        "import",
        "clear-review-dates",
        "--imported-on",
        "2026-08-01",
    ]);
    assert!(said.contains("2026-11-25 (2 notes)"), "{said}");
}

/// A day still to come stamped nothing.
#[test]
fn the_named_days_are_never_ahead() {
    let i = stamped();
    let said = i.h.run(&[
        "--now",
        "2026-11-20T00:00:00Z",
        "import",
        "clear-review-dates",
        "--imported-on",
        "2026-08-01",
    ]);
    assert!(!said.contains("2026-11-25"), "{said}");
}

/// The source had no id, so the date was chosen by its path then; the ledger still says which.
#[test]
fn a_stamped_note_moved_since_is_still_cleared() {
    let i = stamped();
    let from = i.h.vault.path().join("3-permanent/noid.md");
    fs::create_dir_all(i.h.vault.path().join("private")).unwrap();
    fs::rename(from, i.h.vault.path().join("private/moved.md")).unwrap();
    let said = i.clear(&["--apply"]);
    assert!(said.contains("2 review dates cleared"), "{said}");
    assert_eq!(line(&i.copy("private/moved.md"), "review"), None);
}

#[test]
fn a_cleared_note_leaves_the_review_queue() {
    let i = stamped();
    let due = || i.h.run(&["--now", NOW, "due", "--within", "365", "--json"]);
    i.h.run(&["--now", NOW, "index"]);
    assert!(due().contains("3-permanent/byid.md"), "{}", due());
    i.clear(&["--apply"]);
    let left = due();
    assert!(
        !left.contains("byid.md") && !left.contains("noid.md") && left.contains("hand.md"),
        "{left}"
    );
}

/// Wherever a run is killed, nothing is held and every copy is still brain's.
#[test]
fn a_run_killed_anywhere_leaves_every_copy_brains() {
    for (hook, at, dated) in [
        ("BRAIN_TEST_ABORT_BETWEEN_CLEAR_SAVES", "1", 2),
        ("BRAIN_TEST_ABORT_AFTER_CLEARED", "0", 2),
        ("BRAIN_TEST_ABORT_AFTER_CLEARED", "1", 1),
    ] {
        // Left alone: a follow of the sources takes every change.
        let i = stamped();
        assert!(!i.apply_with(Some((hook, at))).status.success(), "{hook}");
        assert_eq!(i.dated(), dated, "{hook} {at}");
        i.both_update();
        // Run again: it finishes, and the same holds.
        let i = stamped();
        assert!(!i.apply_with(Some((hook, at))).status.success(), "{hook}");
        let said = i.clear(&["--apply"]);
        assert!(
            said.contains(&format!("{dated} review date")),
            "{hook}: {said}"
        );
        assert_eq!(i.dated(), 0);
        i.both_update();
    }
}

#[cfg(unix)]
#[test]
fn a_note_that_cannot_be_written_is_named_and_stays_brains() {
    use std::os::unix::fs::PermissionsExt;
    let i = stamped();
    let folder = i.h.vault.path().join("3-permanent");
    let mode = |m| fs::set_permissions(&folder, fs::Permissions::from_mode(m)).unwrap();
    mode(0o500);
    let out = i.apply_with(None);
    mode(0o700);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("could not be written") && said.contains("3-permanent/byid.md"),
        "{said}"
    );
    assert!(said.contains("0 review dates cleared"), "{said}");
    i.both_update();
}

#[cfg(target_os = "macos")]
#[test]
fn a_ledger_that_cannot_be_saved_stops_before_any_note_is_written() {
    let i = stamped();
    let ledger = i.h.home.path().join("data/import-ledger.json");
    let flag = |f: &str| {
        let done = std::process::Command::new("chflags")
            .args([f.as_ref(), ledger.as_os_str()])
            .status()
            .unwrap();
        assert!(done.success());
    };
    flag("uchg");
    let out = i.apply_with(None);
    flag("nouchg");
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && why.contains("run the same command again"),
        "{why}"
    );
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("0 review dates cleared"), "{said}");
    assert_eq!(i.dated(), 2);
    i.both_update();
}

/// A follow pass saving an older copy of the ledger over the run's would undo what it recorded.
#[test]
fn clearing_waits_for_whoever_holds_the_import_lock() {
    let i = stamped();
    let held = Lock::acquire(&i.h.home.path().join("data/import.lock")).unwrap();
    let mut command = i.h.command();
    command
        .args(CLEAR)
        .arg("--apply")
        .stdout(std::process::Stdio::null());
    let mut running = command.spawn().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(i.dated(), 2);
    assert!(running.try_wait().unwrap().is_none(), "it did not wait");
    // What the holder saves is what the run reads.
    let path = i.h.home.path().join("data/import-ledger.json");
    let mut ledger: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    ledger["vault/theirs.md"] =
        serde_json::json!({ "id": null, "src_hash": "", "dst_hash": "theirs" });
    fs::write(&path, ledger.to_string()).unwrap();
    drop(held);
    assert!(running.wait().unwrap().success());
    assert_eq!(i.dated(), 0);
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("vault/theirs.md"), "{after}");
}

#[test]
fn the_import_lock_is_held_while_notes_are_written() {
    let i = stamped();
    let lock = i.h.home.path().join("data/import.lock");
    let out = i.apply_while(|| assert!(Lock::try_acquire(&lock).unwrap().is_none()));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(i.dated(), 0);
}

#[test]
fn a_note_saved_while_clearing_keeps_its_words_and_fails_the_run() {
    let i = stamped();
    let rel = "3-permanent/noid.md";
    let theirs = format!("{}new words\n", i.copy(rel));
    let out = i.apply_while(|| fs::write(i.h.vault.path().join(rel), &theirs).unwrap());
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{said}");
    assert_eq!(i.copy(rel), theirs);
    assert!(said.contains("1 review date cleared"), "{said}");
    assert!(
        said.contains("run it again:\n  3-permanent/noid.md"),
        "{said}"
    );
}

/// A note gone since it was read is for the next run to look at; one brain may not read is for
/// the person to put right.
#[cfg(unix)]
#[test]
fn a_note_that_could_not_be_written_is_told_from_one_that_moved() {
    use std::os::unix::fs::PermissionsExt;
    let i = stamped();
    let gone = i.h.vault.path().join("3-permanent/byid.md");
    let locked = i.h.vault.path().join("3-permanent/noid.md");
    let out = i.apply_while(|| {
        fs::remove_file(&gone).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    });
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o600)).unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("run it again:\n  3-permanent/byid.md"),
        "{said}"
    );
    assert!(
        said.contains("then run it again:\n  3-permanent/noid.md: "),
        "{said}"
    );
}

/// An import wrote the bare date: anything added to the line is the person's, and stays.
#[test]
fn a_review_line_a_person_added_to_is_left() {
    let i = stamped();
    let rel = "3-permanent/byid.md";
    let original = i.copy(rel);
    let bare = format!("review: {BY_ID}\n");
    for theirs in [
        format!("review: {BY_ID} # I checked this one, keep\n"),
        format!("review: {BY_ID} (confirmed with the supplier)\n"),
        format!("review: {BY_ID}, then again in June\n"),
        format!("review: {BY_ID}T09:30:00\n"),
        format!("review: \"{BY_ID}\"\n"),
        format!("review: {BY_ID} \n"),
        format!("review:  {BY_ID}\n"),
    ] {
        let text = original.replace(&bare, &theirs);
        assert_ne!(text, original);
        fs::write(i.h.vault.path().join(rel), &text).unwrap();
        let said = i.clear(&[]);
        assert!(
            said.contains("1 review date would be cleared"),
            "{theirs}: {said}"
        );
        assert!(said.contains("3 set some other way"), "{theirs}: {said}");
    }
    // A line carried on to the next is not even a date.
    let theirs = original.replace(&bare, &format!("{bare}  and again\n"));
    fs::write(i.h.vault.path().join(rel), &theirs).unwrap();
    i.clear(&["--apply"]);
    assert_eq!(i.copy(rel), theirs);
}

/// A copy saved with Windows line ends is still the line an import wrote.
#[test]
fn a_stamped_line_ending_in_crlf_is_cleared() {
    let i = stamped();
    let rel = "3-permanent/byid.md";
    let crlf = i.copy(rel).replace('\n', "\r\n");
    fs::write(i.h.vault.path().join(rel), &crlf).unwrap();
    i.clear(&["--apply"]);
    assert_eq!(
        i.copy(rel),
        crlf.replace(&format!("review: {BY_ID}\r\n"), "")
    );
}

/// The notes are written by then: what was done is said before the index's trouble.
#[test]
fn an_index_that_cannot_be_updated_still_leaves_the_report() {
    let i = stamped();
    let data = i.h.home.path().join("data");
    for name in ["index.db", "index.db-wal", "index.db-shm"] {
        let _ = fs::remove_file(data.join(name));
    }
    fs::create_dir(data.join("index.db")).unwrap();
    let out = i.apply_with(None);
    let said = String::from_utf8_lossy(&out.stdout);
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("2 review dates cleared"), "{said}{why}");
    assert!(why.contains("brain index"), "{why}");
}

/// No import ran on a day still to come, so naming one could only take a date set by hand.
#[test]
fn a_day_still_to_come_is_refused() {
    let i = stamped();
    let out =
        i.h.brain()
            .args([
                "--now",
                NOW,
                "import",
                "clear-review-dates",
                "--imported-on",
                "2026-11-26",
                "--apply",
            ])
            .output()
            .unwrap();
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && why.contains("2026-11-26"), "{why}");
    assert_eq!(i.dated(), 2);
}

#[test]
fn nothing_to_clear_asks_for_no_commit() {
    let i = stamped();
    i.clear(&["--apply"]);
    let said = i.clear(&[]);
    assert!(said.contains("0 review dates would be cleared"), "{said}");
    assert!(!said.contains("commit your vault"), "{said}");
}

#[cfg(unix)]
#[test]
fn everything_not_looked_at_is_listed() {
    let i = stamped();
    for n in 0..25 {
        std::os::unix::fs::symlink(
            i.h.vault.path().join("hand.md"),
            i.h.vault.path().join(format!("link{n:02}.md")),
        )
        .unwrap();
    }
    let said = i.clear(&[]);
    assert!(said.contains("link24.md"), "{said}");
}

#[cfg(unix)]
#[test]
fn a_vault_that_cannot_be_read_is_an_error() {
    use std::os::unix::fs::PermissionsExt;
    let i = stamped();
    let mode = |m| fs::set_permissions(i.h.vault.path(), fs::Permissions::from_mode(m)).unwrap();
    mode(0o000);
    let out =
        i.h.brain()
            .current_dir(i.h.home.path())
            .args(CLEAR)
            .output()
            .unwrap();
    mode(0o700);
    let why = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && why.contains("cannot be read"),
        "{out:?}"
    );
}

/// A read-only note is replaced like any other, and keeps its time too.
#[cfg(unix)]
#[test]
fn a_read_only_note_keeps_its_time() {
    use std::os::unix::fs::PermissionsExt;
    let i = stamped();
    let at = i.h.vault.path().join("3-permanent/byid.md");
    let before = fs::metadata(&at).unwrap().modified().unwrap();
    fs::set_permissions(&at, fs::Permissions::from_mode(0o444)).unwrap();
    i.clear(&["--apply"]);
    assert_eq!(i.dated(), 0);
    assert_eq!(fs::metadata(&at).unwrap().modified().unwrap(), before);
}

/// The rollback check refuses after a run cut short, and passes once it has run again.
#[cfg(unix)]
#[test]
fn rolling_back_is_refused_until_the_clearing_has_finished() {
    let i = stamped();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/clearing-rollback-check.py");
    let check = || {
        std::process::Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg("--data")
            .arg(i.h.home.path().join("data"))
            .arg("--config")
            .arg(i.h.home.path().join("config/config.toml"))
            .output()
            .unwrap()
    };
    let killed = i.apply_with(Some(("BRAIN_TEST_ABORT_AFTER_CLEARED", "1")));
    assert!(!killed.status.success());
    let out = check();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success() && said.contains("still holds") && said.contains("3-permanent/"),
        "{out:?}"
    );
    i.clear(&["--apply"]);
    let out = check();
    assert!(out.status.success(), "{out:?}");
}
