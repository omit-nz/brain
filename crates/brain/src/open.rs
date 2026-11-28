//! The recent notes with something left open, which `due` names beside the review dates. A heading
//! is the only signal and nothing records that an item was settled, so each is named as one brain
//! cannot vouch for; a correction takes it off for good.

use std::path::Path;

use brain_index::{Index, Written};
use brain_note::{Split, Vault, prose};
use jiff::ToSpan;
use jiff::civil::Date;

/// How far back a note is looked at.
const DAYS: i64 = 14;
pub const SHOWN: usize = 5;

pub fn heading() -> String {
    format!(
        "Written in the last {DAYS} days with something left open. brain cannot tell whether it \
         has been settled since:"
    )
}

/// The recent notes with something left open, and how many recent ones could not be read to tell.
pub struct Open {
    pub notes: Vec<Written>,
    pub unread: usize,
}

impl Open {
    /// What to say of the notes that could not be read.
    pub fn unread_line(&self) -> Option<String> {
        match self.unread {
            0 => None,
            1 => Some("1 recent note could not be read.".into()),
            n => Some(format!("{n} recent notes could not be read.")),
        }
    }
}

/// `owner`'s kept notes of the last two weeks with a section headed open or waiting that nothing
/// overtakes, newest first. Copies of an AI tool's memory are never read.
pub fn notes(index: &Index, vault: &Vault, owner: &str, today: Date) -> anyhow::Result<Open> {
    let since = today.checked_sub(DAYS.days())?;
    let mut notes = index.written_since(owner, since, today)?;
    let mut unread = 0;
    notes.retain(|n| {
        let path = Path::new(&n.path);
        if vault.layout.is_memory(path) {
            return false;
        }
        match vault.read(path) {
            Ok(text) => left_open(Split::parse(&text).body),
            Err(_) => {
                // One gone since the index was made holds nothing any more.
                let gone = matches!(vault.root.join(path).try_exists(), Ok(false));
                unread += usize::from(!gone);
                false
            }
        }
    });
    Ok(Open { notes, unread })
}

/// Whether a heading outside code holds the word `open` or `waiting`.
fn left_open(body: &str) -> bool {
    prose(body).any(|line| {
        let head = line.trim_start_matches(' ');
        line.len() - head.len() <= 3
            && head
                .strip_prefix('#')
                .is_some_and(|rest| rest.trim_start_matches('#').starts_with([' ', '\t']))
            && head.split(|c: char| !c.is_alphanumeric()).any(|word| {
                ["open", "waiting"]
                    .iter()
                    .any(|w| word.eq_ignore_ascii_case(w))
            })
    })
}
