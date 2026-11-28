//! Taking out the review dates 0.2.5 and earlier stamped on the notes they imported. A review date
//! is a person's to set, so no copy gets one now. An import's is known by its arithmetic: the day
//! it ran plus 1 to 90 days chosen by a hash of the note's id, or of its source's key when the
//! source had none.
//!
//! A run may stop anywhere: each record is saved accepting both the bytes there and the bytes to
//! come before a note is written, so nothing is ever left half done, and running it again is a
//! plain run. The caller holds the import lock.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use brain_note::{Entry, Split, Vault, rel_key, text_of};
use brain_platform::fs::{Privacy, replace_unchanged, set_modified};
use jiff::ToSpan;
use jiff::civil::Date;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Archive, Ledger, hash};
use crate::config::REVIEW_DAYS;

/// The days an import spread its first reviews over.
const SPREAD: u64 = 90;
/// Notes recorded ahead of their writing at a time.
const BATCH: usize = 200;

/// How many days after an import `seed`'s review fell.
fn offset(seed: &str) -> i64 {
    let h = Sha256::digest(seed.as_bytes());
    (u64::from_le_bytes(h[..8].try_into().expect("8 bytes")) % SPREAD) as i64 + 1
}

/// One note to take a date out of: what it read, and what it is to read.
pub struct Change {
    rel: PathBuf,
    raw: String,
    text: String,
}

impl Change {
    pub fn path(&self) -> String {
        rel_key(&self.rel)
    }
}

/// What a clearing would do with every note holding a review date.
#[derive(Default)]
pub struct Plan {
    /// The notes whose date an import wrote.
    pub changes: Vec<Change>,
    /// Captures, whose date was agreed to with the note.
    pub captured: usize,
    /// Dates no import of the days given wrote.
    pub other: usize,
    /// What could not be looked at, and why.
    pub skipped: Vec<(String, String)>,
    /// For the dates left, the other days an import would have written them on, and how many.
    pub days: BTreeMap<Date, usize>,
}

/// Reads every note, the archived copies too, for a date an import on one of `days` wrote.
/// Nothing is written.
pub fn plan(vault: &Vault, ledger: &Ledger, archive: &Archive, days: &[Date], today: Date) -> Plan {
    let mut out = Plan::default();
    let mut notes = Vec::new();
    for entry in vault.walk() {
        match entry {
            Entry::Note { rel, .. } => notes.push(rel),
            Entry::Skipped { rel, why } => out.skipped.push((rel_key(&rel), why.describe())),
        }
    }
    // An archived file that is not there has nothing to clear; one that cannot be reached is
    // named with the rest not looked at.
    let archived = archive.files(&vault.root).filter(|rel| {
        !std::fs::symlink_metadata(vault.root.join(rel))
            .is_err_and(|e| e.kind() == ErrorKind::NotFound)
    });
    notes.extend(archived.map(Path::to_path_buf));
    for rel in notes {
        let raw = match vault.read(&rel) {
            Ok(raw) => raw,
            Err(why) => {
                out.skipped.push((rel_key(&rel), why.describe()));
                continue;
            }
        };
        let mut split = Split::parse(&raw);
        let Ok(Some(fm)) = split.fm.as_mut() else {
            continue;
        };
        let date = |key: &str| text_of(fm.get(key))?.get(..10)?.parse::<Date>().ok();
        let Some(review) = date("review") else {
            continue;
        };
        // An import wrote the bare date: anything added to the line is the person's.
        if !fm.holds_only("review", &review.to_string()) {
            out.other += 1;
            continue;
        }
        // The id as brain writes one, which an import's choice was made by.
        let id = text_of(fm.get("id"))
            .and_then(|id| Uuid::parse_str(&id).ok())
            .map(|id| id.to_string());
        // The sources it was imported from, archived or not.
        let keys: Vec<&str> = id.as_deref().map_or_else(Vec::new, |id| {
            ledger
                .keys_of(&vault.root, id)
                .chain(archive.keys_of(&vault.root, id))
                .collect()
        });
        let known = !keys.is_empty();
        // A capture is dated the same way every time, and no import's record knows it.
        let captured = date("date").and_then(|d| d.checked_add(REVIEW_DAYS.days()).ok());
        if !known && captured == Some(review) {
            out.captured += 1;
            continue;
        }
        // A memory copy was dated by its id alone. A vault note with no source id was dated by
        // its path then, which a moved note's record still says.
        let here = format!("vault/{}", rel_key(&rel));
        let memory = keys.iter().any(|key| key.starts_with("memory/"));
        let would: BTreeSet<Date> = id
            .as_deref()
            .into_iter()
            .chain(keys.into_iter().filter(|key| key.starts_with("vault/")))
            .chain((!memory).then_some(here.as_str()))
            .filter_map(|seed| review.checked_sub(offset(seed).days()).ok())
            .collect();
        if !days.iter().any(|d| would.contains(d)) {
            out.other += 1;
            // Named only for what brain wrote: a day named for a person's own note clears it.
            for day in would.into_iter().filter(|d| known && *d <= today) {
                *out.days.entry(day).or_default() += 1;
            }
            continue;
        }
        fm.remove("review");
        let text = split.render();
        out.changes.push(Change { rel, raw, text });
    }
    out
}

/// A note that was to be written and was not.
pub enum Left {
    /// Saved, moved or removed since it was read: the next run takes what is there.
    Saved(String),
    /// It could not be written, and why.
    Refused(String, String),
}

/// A test that kills a run part-way sets one of these; release builds never read them.
fn abort_at(hook: &str, reached: usize) {
    let at = std::env::var(hook).ok().and_then(|n| n.parse().ok());
    if cfg!(debug_assertions) && at == Some(reached) {
        std::process::abort();
    }
}

/// A test that acts while a run is part-way names a file here: the run waits until it is gone.
fn wait_at_gate() {
    let gate = std::env::var_os("BRAIN_TEST_CLEAR_GATE").filter(|_| cfg!(debug_assertions));
    while gate.as_ref().is_some_and(|gate| Path::new(gate).exists()) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Writes each change that its note still reads as planned, leaving the file's time, and returns
/// the notes left. Each batch's records are saved before any of its notes is written, the ledger
/// first.
pub fn write(
    vault: &Vault,
    changes: &[Change],
    ledger: &mut Ledger,
    archive: &mut Archive,
    save_ledger: &mut dyn FnMut(&Ledger) -> anyhow::Result<()>,
    save_archive: &mut dyn FnMut(&Archive) -> anyhow::Result<()>,
) -> anyhow::Result<Vec<Left>> {
    let (mut left, mut written) = (Vec::new(), 0);
    for batch in changes.chunks(BATCH) {
        let (mut in_ledger, mut in_archive) = (false, false);
        for c in batch {
            let (from, to) = (hash(c.raw.as_bytes()), hash(c.text.as_bytes()));
            in_ledger |= ledger.rewrite(&vault.root, &from, &to);
            in_archive |= archive.rewrite(&vault.root, &from, &to);
        }
        if in_ledger {
            save_ledger(ledger)?;
        }
        abort_at("BRAIN_TEST_ABORT_BETWEEN_CLEAR_SAVES", 1);
        if in_archive {
            save_archive(archive)?;
        }
        abort_at("BRAIN_TEST_ABORT_AFTER_CLEARED", 0);
        wait_at_gate();
        for c in batch {
            let at = std::fs::metadata(vault.root.join(&c.rel)).and_then(|m| m.modified());
            let done = replace_unchanged(
                &vault.root,
                &c.rel,
                |now| now == Some(c.raw.as_bytes()),
                c.text.as_bytes(),
                Privacy::Normal,
            );
            if let Err(e) = done {
                let changed = match std::fs::read(vault.root.join(&c.rel)) {
                    Ok(now) => now != c.raw.as_bytes(),
                    Err(why) => why.kind() == ErrorKind::NotFound,
                };
                left.push(if changed {
                    Left::Saved(c.path())
                } else {
                    Left::Refused(c.path(), e.to_string())
                });
                continue;
            }
            if let Ok(at) = at {
                let _ = set_modified(&vault.root, &c.rel, at);
            }
            written += 1;
            abort_at("BRAIN_TEST_ABORT_AFTER_CLEARED", written);
        }
    }
    Ok(left)
}

#[cfg(test)]
mod tests {
    use brain_platform::fs as platform;

    use super::*;

    const ID: &str = "0195f2e0-0000-7000-8000-000000000009";

    fn note(dir: &Path, name: &str, text: &str) {
        platform::write_atomic(dir, Path::new(name), text.as_bytes(), Privacy::Normal).unwrap();
    }

    fn recorded(key: &str, vault: Option<&Path>, dst: &str) -> Ledger {
        let entry = serde_json::json!({
            "id": ID, "src_hash": "", "dst_hash": dst, "vault": vault
        });
        serde_json::from_value(serde_json::json!({ key: entry })).unwrap()
    }

    fn stored(ledger: &Ledger) -> serde_json::Value {
        serde_json::to_value(ledger).unwrap()
    }

    fn run(vault: &Vault, changes: &[Change], ledger: &mut Ledger) -> Vec<Left> {
        let none = &mut Archive::default();
        write(vault, changes, ledger, none, &mut |_| Ok(()), &mut |_| {
            Ok(())
        })
        .unwrap()
    }

    fn change(rel: &str, raw: &str, text: &str) -> Change {
        Change {
            rel: rel.into(),
            raw: raw.into(),
            text: text.into(),
        }
    }

    const STAMPED: &str = "---\nreview: 2027-01-01\n---\nsame\n";
    const CLEARED: &str = "---\n---\nsame\n";

    /// A day named for a note brain never imported would clear a date set by hand.
    #[test]
    fn days_are_named_only_for_what_brain_wrote() {
        let dir = tempfile::tempdir().unwrap();
        note(
            dir.path(),
            "hand.md",
            "---\nreview: 2026-12-01\n---\nmine\n",
        );
        let vault = Vault::open(dir.path()).unwrap();
        let today = jiff::civil::date(2027, 1, 1);
        let plan = plan(&vault, &Ledger::default(), &Archive::default(), &[], today);
        assert_eq!((plan.other, plan.days.len()), (1, 0));
    }

    /// A memory copy's date was chosen by its id: neither its key nor its path ever stamped it.
    #[test]
    fn a_memory_copy_is_dated_by_its_id_alone() {
        let day = jiff::civil::date(2026, 11, 25);
        let path = "working-memory/-proj/one.md";
        for seed in ["memory/-proj/one.md".to_string(), format!("vault/{path}")] {
            let dir = tempfile::tempdir().unwrap();
            let review = day.checked_add(offset(&seed).days()).unwrap();
            // A fixed id, whose own offset is not this seed's.
            assert_ne!(offset(ID), offset(&seed));
            note(
                dir.path(),
                path,
                &format!("---\nid: {ID}\nreview: {review}\n---\nmine\n"),
            );
            let vault = Vault::open(dir.path()).unwrap();
            let ledger = recorded("memory/-proj/one.md", None, "");
            let plan = plan(&vault, &ledger, &Archive::default(), &[day], day);
            assert_eq!(plan.changes.len(), 0, "{seed}");
        }
    }

    /// Records from before the ledger kept a vault count as this one.
    #[test]
    fn a_record_naming_no_vault_is_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        note(dir.path(), "a.md", STAMPED);
        let vault = Vault::open(dir.path()).unwrap();
        let mut ledger = recorded("vault/a.md", None, &hash(STAMPED.as_bytes()));
        run(&vault, &[change("a.md", STAMPED, CLEARED)], &mut ledger);
        assert_eq!(
            stored(&ledger)["vault/a.md"]["dst_hash"],
            hash(CLEARED.as_bytes())
        );
    }

    /// Two copies hold the same bytes and only one can be written: both stay brain's, whichever
    /// record is looked at.
    #[test]
    fn twins_stay_brains_whichever_is_written() {
        for (written, refused) in [("a.md", "b.md"), ("b.md", "a.md")] {
            let dir = tempfile::tempdir().unwrap();
            note(dir.path(), written, STAMPED);
            // Saved by the person since it was read.
            note(dir.path(), refused, "changed\n");
            let vault = Vault::open(dir.path()).unwrap();
            let mut ledger = recorded("vault/a.md", None, &hash(STAMPED.as_bytes()));
            ledger
                .0
                .extend(recorded("vault/b.md", None, &hash(STAMPED.as_bytes())).0);
            let changes = [
                change("a.md", STAMPED, CLEARED),
                change("b.md", STAMPED, CLEARED),
            ];
            let left = run(&vault, &changes, &mut ledger);
            assert!(matches!(&left[..], [Left::Saved(path)] if path == refused));
            for key in ["vault/a.md", "vault/b.md"] {
                let record = &stored(&ledger)[key];
                assert_eq!(record["dst_hash"], hash(CLEARED.as_bytes()), "{key}");
                assert_eq!(record["was"], hash(STAMPED.as_bytes()), "{key}");
            }
        }
    }

    /// A record that matched only by what it was keeps both of its hashes.
    #[test]
    fn a_record_that_has_moved_on_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        note(dir.path(), "a.md", STAMPED);
        let vault = Vault::open(dir.path()).unwrap();
        let mut ledger: Ledger = serde_json::from_value(serde_json::json!({
            "vault/a.md": { "id": ID, "src_hash": "", "dst_hash": "other",
                            "was": hash(STAMPED.as_bytes()) }
        }))
        .unwrap();
        run(&vault, &[change("a.md", STAMPED, CLEARED)], &mut ledger);
        assert_eq!(stored(&ledger)["vault/a.md"]["dst_hash"], "other");
    }

    /// Every batch's records are saved before its notes are written, also past the first.
    #[test]
    fn every_batch_is_recorded_before_it_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let text = |n: usize| format!("---\nreview: 2027-01-01\n---\n{n}\n");
        let cleared = |n: usize| format!("---\n---\n{n}\n");
        let mut ledger = Ledger::default();
        let changes: Vec<Change> = (0..BATCH + 5)
            .map(|n| {
                let rel = format!("{n}.md");
                note(dir.path(), &rel, &text(n));
                let key = format!("vault/{rel}");
                ledger
                    .0
                    .extend(recorded(&key, None, &hash(text(n).as_bytes())).0);
                change(&rel, &text(n), &cleared(n))
            })
            .collect();
        let before = stored(&ledger);
        let back =
            |held: &serde_json::Value| -> Ledger { serde_json::from_value(held.clone()).unwrap() };
        let vault = Vault::open(dir.path()).unwrap();
        // Whole, and then with the second batch's save failing.
        for fail_at in [0, 2] {
            for (n, c) in changes.iter().enumerate() {
                note(dir.path(), &c.path(), &text(n));
            }
            ledger = back(&before);
            let (mut saves, mut saved) = (0, before.clone());
            let done = write(
                &vault,
                &changes,
                &mut ledger,
                &mut Archive::default(),
                &mut |l| {
                    saves += 1;
                    anyhow::ensure!(saves != fail_at, "no room");
                    saved = stored(l);
                    Ok(())
                },
                &mut |_| Ok(()),
            );
            assert_eq!(done.is_ok(), fail_at == 0);
            let saved = back(&saved);
            for c in &changes {
                let now = std::fs::read(dir.path().join(c.path())).unwrap();
                let entry = &saved.0[&format!("vault/{}", c.path())];
                assert!(entry.copy.accepts(&hash(&now)), "{}", c.path());
            }
        }
    }

    /// The person may save a note between the reading and the writing of it.
    #[test]
    fn a_note_saved_meanwhile_keeps_its_words() {
        let dir = tempfile::tempdir().unwrap();
        let newer = "---\nreview: 2027-01-01\n---\nnew words\n";
        note(dir.path(), "a.md", newer);
        let vault = Vault::open(dir.path()).unwrap();
        let left = run(
            &vault,
            &[change("a.md", STAMPED, CLEARED)],
            &mut Ledger::default(),
        );
        assert!(matches!(&left[..], [Left::Saved(_)]));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
            newer
        );
    }
}
