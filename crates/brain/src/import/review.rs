//! Taking out the review dates 0.2.5 and earlier stamped on the notes they imported. A review date
//! is a person's to set, so no copy gets one now. An import's is known by its arithmetic: the day
//! it ran plus 1 to 90 days chosen by a hash of the note's id, or of its source's key when the
//! source had none.

use brain_note::{Entry, Split, Vault, rel_key, text_of};
use brain_platform::fs::{Privacy, replace_unchanged, set_modified};
use jiff::ToSpan;
use jiff::civil::Date;
use sha2::{Digest, Sha256};

use super::{Ledger, hash};
use crate::config::REVIEW_DAYS;

/// The days an import spread its first reviews over.
const SPREAD: u64 = 90;

fn stamped(day: Date, seed: &str) -> Option<Date> {
    let h = Sha256::digest(seed.as_bytes());
    let n = u64::from_le_bytes(h[..8].try_into().expect("8 bytes")) % SPREAD;
    day.checked_add((n as i64 + 1).days()).ok()
}

/// What became of every note holding a review date.
#[derive(Debug, Default)]
pub struct Cleared {
    /// The notes whose date an import wrote.
    pub notes: Vec<String>,
    /// Captures, whose date was agreed to with the note.
    pub captured: usize,
    /// Dates no import of those days wrote.
    pub other: usize,
    /// Notes whose date an import wrote that could not be written back.
    pub failed: Vec<String>,
}

/// Takes the `review` line out of every note whose date an import on one of `days` wrote, leaving
/// its other bytes and its file time. A copy the ledger knows stays known as brain's. With
/// `apply` unset nothing is written.
pub fn clear(vault: &Vault, ledger: &mut Ledger, days: &[Date], apply: bool) -> Cleared {
    let mut out = Cleared::default();
    for entry in vault.walk() {
        let Entry::Note { rel, .. } = entry else {
            continue;
        };
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
        // A capture is dated the same way every time, and no import's ledger knows it.
        let id = text_of(fm.get("id"));
        let known = id
            .as_deref()
            .is_some_and(|id| ledger.copied(&vault.root, id));
        if !known
            && date("date").and_then(|d| d.checked_add(REVIEW_DAYS.days()).ok()) == Some(review)
        {
            out.captured += 1;
            continue;
        }
        let path = rel_key(&rel);
        let seeds = [id, Some(format!("vault/{path}"))];
        let imported = seeds
            .iter()
            .flatten()
            .any(|seed| days.iter().any(|d| stamped(*d, seed) == Some(review)));
        if !imported {
            out.other += 1;
            continue;
        }
        if apply {
            fm.remove("review");
            let text = split.render();
            let at = std::fs::metadata(vault.root.join(&rel)).and_then(|m| m.modified());
            let written = replace_unchanged(
                &vault.root,
                &rel,
                |now| now == Some(raw.as_bytes()),
                text.as_bytes(),
                Privacy::Normal,
            );
            if written.is_err() {
                out.failed.push(path);
                continue;
            }
            if let Ok(at) = at {
                let _ = set_modified(&vault.root, &rel, at);
            }
            ledger.rewritten(&vault.root, &hash(raw.as_bytes()), &hash(text.as_bytes()));
        }
        out.notes.push(path);
    }
    out
}
