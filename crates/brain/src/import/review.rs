//! Taking out the review dates 0.2.5 and earlier stamped on the notes they imported. A review date
//! is a person's to set, so no copy gets one now. An import's is known by its arithmetic: the day
//! it ran plus 1 to 90 days chosen by a hash of the note's id, or of its source's key when the
//! source had none.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use brain_note::{Entry, Split, Vault, rel_key, text_of};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, replace_unchanged, set_modified, write_atomic};
use jiff::ToSpan;
use jiff::civil::Date;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{Archive, Ledger, hash};
use crate::config::REVIEW_DAYS;

/// The days an import spread its first reviews over.
const SPREAD: u64 = 90;
/// Notes recorded in the ledger at a time, ahead of their writing.
const BATCH: usize = 200;
/// The days of a clearing under way, `[]` once it is done. While one is cut short, some copies are
/// recorded as cleared and are not yet written, which anything else reading the ledger would take
/// for the person's own edits.
const UNFINISHED: &str = "clearing.json";

/// The days of a clearing that did not finish.
pub fn unfinished(dirs: &Dirs) -> anyhow::Result<Vec<Date>> {
    let path = dirs.data.join(UNFINISHED);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// Records a clearing on `days` as under way, or with none as done.
pub fn mark(dirs: &Dirs, days: &[Date]) -> anyhow::Result<()> {
    let text = serde_json::to_string(days)?;
    write_atomic(
        &dirs.data,
        Path::new(UNFINISHED),
        text.as_bytes(),
        Privacy::Private,
    )?;
    Ok(())
}

/// Refuses to go on over a ledger a clearing left half recorded.
pub fn refuse_if_unfinished(dirs: &Dirs) -> anyhow::Result<()> {
    let days = unfinished(dirs)?;
    let named: String = days.iter().map(|d| format!(" --imported-on {d}")).collect();
    anyhow::ensure!(
        days.is_empty(),
        "a clearing of review dates was cut short, so nothing is imported until it is finished: run `brain import clear-review-dates{named} --apply`"
    );
    Ok(())
}

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
    /// For the dates left, the other days an import would have written them on, and how many.
    pub days: BTreeMap<Date, usize>,
}

/// Reads every note, the archived copies too, for a date an import on one of `days` wrote.
/// Nothing is written.
pub fn plan(vault: &Vault, ledger: &Ledger, archive: &Archive, days: &[Date], today: Date) -> Plan {
    let mut out = Plan::default();
    let notes = vault.walk().filter_map(|entry| match entry {
        Entry::Note { rel, .. } => Some(rel),
        Entry::Skipped { .. } => None,
    });
    let archived = archive.files(&vault.root).map(Path::to_path_buf);
    for rel in notes.chain(archived) {
        let Ok(raw) = vault.read(&rel) else {
            continue;
        };
        let mut split = Split::parse(&raw);
        let Ok(Some(fm)) = split.fm.as_mut() else {
            continue;
        };
        let date = |key: &str| text_of(fm.get(key))?.get(..10)?.parse::<Date>().ok();
        let Some(review) = date("review") else {
            continue;
        };
        // The id as brain writes one, which an import's choice was made by.
        let id = text_of(fm.get("id"))
            .and_then(|id| Uuid::parse_str(&id).ok())
            .map(|id| id.to_string());
        // The sources it was imported from: a note moved since was dated by its path then.
        let keys: Vec<&str> = id
            .as_deref()
            .map(|id| ledger.keys_of(&vault.root, id).collect())
            .unwrap_or_default();
        // A capture is dated the same way every time, and no import's record knows it.
        let known = !keys.is_empty() || id.as_deref().is_some_and(|id| archive.names(id));
        let captured = date("date").and_then(|d| d.checked_add(REVIEW_DAYS.days()).ok());
        if !known && captured == Some(review) {
            out.captured += 1;
            continue;
        }
        let here = format!("vault/{}", rel_key(&rel));
        let would: BTreeSet<Date> = id
            .as_deref()
            .into_iter()
            .chain(keys)
            .chain([here.as_str()])
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

/// A test that kills a run part-way sets this; release builds never read it.
fn abort_after() -> Option<usize> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("BRAIN_TEST_ABORT_AFTER_CLEARED")
        .ok()?
        .parse()
        .ok()
}

/// Writes each change that its note still reads as planned, leaving the file's time, and returns
/// the notes that could not be written. A copy the ledger knows stays known as brain's: it is
/// recorded and saved before it is written, so a run cut short is finished by the next, which
/// writes the same bytes.
pub fn write(
    vault: &Vault,
    changes: &[Change],
    ledger: &mut Ledger,
    archive: &mut Archive,
    save: &mut dyn FnMut(&Ledger, &Archive) -> anyhow::Result<()>,
) -> anyhow::Result<Vec<String>> {
    let record = |ledger: &mut Ledger, archive: &mut Archive, from: &str, to: &str| {
        let (from, to) = (hash(from.as_bytes()), hash(to.as_bytes()));
        ledger.rewritten(&vault.root, &from, &to);
        archive.rewritten(&vault.root, &from, &to);
    };
    let (mut failed, mut written) = (Vec::new(), 0);
    for batch in changes.chunks(BATCH) {
        for c in batch {
            record(ledger, archive, &c.raw, &c.text);
        }
        save(ledger, archive)?;
        let (mut unwritten, mut done_from) = (Vec::new(), HashSet::new());
        for c in batch {
            let at = std::fs::metadata(vault.root.join(&c.rel)).and_then(|m| m.modified());
            let done = replace_unchanged(
                &vault.root,
                &c.rel,
                |now| now == Some(c.raw.as_bytes()),
                c.text.as_bytes(),
                Privacy::Normal,
            );
            if done.is_err() {
                unwritten.push(c);
                failed.push(c.path());
                continue;
            }
            done_from.insert(c.raw.as_str());
            if let Ok(at) = at {
                let _ = set_modified(&vault.root, &c.rel, at);
            }
            written += 1;
            if abort_after() == Some(written) {
                std::process::abort();
            }
        }
        if !unwritten.is_empty() {
            // Records go by content, so one a written twin shares stays as written.
            for c in unwritten {
                if !done_from.contains(c.raw.as_str()) {
                    record(ledger, archive, &c.text, &c.raw);
                }
            }
            save(ledger, archive)?;
        }
    }
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use brain_platform::fs as platform;

    use super::*;

    fn note(dir: &Path, name: &str, text: &str) {
        platform::write_atomic(dir, Path::new(name), text.as_bytes(), Privacy::Normal).unwrap();
    }

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

    #[test]
    fn a_copy_written_stays_recorded_when_its_twin_cannot_be() {
        let dir = tempfile::tempdir().unwrap();
        let (raw, text) = ("---\nreview: 2027-01-01\n---\nsame\n", "---\n---\nsame\n");
        note(dir.path(), "a.md", raw);
        // Saved by the person since it was read.
        note(dir.path(), "b.md", "changed\n");
        let vault = Vault::open(dir.path()).unwrap();
        let change = |rel: &str| Change {
            rel: rel.into(),
            raw: raw.into(),
            text: text.into(),
        };
        let entry: super::super::LedgerEntry = serde_json::from_value(serde_json::json!({
            "id": null, "src_hash": "", "dst_hash": hash(raw.as_bytes())
        }))
        .unwrap();
        let mut ledger = Ledger([("vault/a.md".to_string(), entry)].into());
        let failed = write(
            &vault,
            &[change("a.md"), change("b.md")],
            &mut ledger,
            &mut Archive::default(),
            &mut |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(failed, ["b.md"]);
        assert_eq!(ledger.0["vault/a.md"].dst_hash, hash(text.as_bytes()));
    }

    /// The person may save a note between the reading and the writing of it.
    #[test]
    fn a_note_saved_meanwhile_keeps_its_words() {
        let dir = tempfile::tempdir().unwrap();
        let put = |text: &str| {
            platform::write_atomic(
                dir.path(),
                Path::new("a.md"),
                text.as_bytes(),
                Privacy::Normal,
            )
            .unwrap();
        };
        let at = dir.path().join("a.md");
        put("---\nreview: 2027-01-01\n---\nold\n");
        let vault = Vault::open(dir.path()).unwrap();
        let change = Change {
            rel: "a.md".into(),
            raw: std::fs::read_to_string(&at).unwrap(),
            text: "---\n---\nold\n".into(),
        };
        put("---\nreview: 2027-01-01\n---\nnew words\n");
        let failed = write(
            &vault,
            &[change],
            &mut Ledger::default(),
            &mut Archive::default(),
            &mut |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(failed, ["a.md"]);
        assert!(
            std::fs::read_to_string(&at)
                .unwrap()
                .ends_with("new words\n")
        );
    }
}
