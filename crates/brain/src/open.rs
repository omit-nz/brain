//! The recent notes with something left open, which `due` names beside the review dates. A heading
//! is the only signal and nothing records that an item was settled, so each is named as one brain
//! cannot vouch for; a correction takes it off for good.

use std::path::Path;

use brain_index::{Index, Written};
use brain_note::{Split, Vault};
use jiff::ToSpan;
use jiff::civil::Date;

/// How far back a note is looked at.
const DAYS: i64 = 14;
pub const SHOWN: usize = 5;
pub const HEADING: &str = "Written in the last 14 days with something left open. brain cannot tell \
                           whether it has been settled since:";

/// `owner`'s kept notes of the last two weeks with a section headed open or waiting that nothing
/// overtakes, newest first. Copies of an AI tool's memory are never read.
pub fn notes(
    index: &Index,
    vault: &Vault,
    owner: &str,
    today: Date,
) -> anyhow::Result<Vec<Written>> {
    let since = today.checked_sub(DAYS.days())?;
    let mut notes = index.written_since(owner, since, today)?;
    notes.retain(|n| {
        let path = Path::new(&n.path);
        !vault.layout.is_memory(path)
            && vault
                .read(path)
                .is_ok_and(|text| left_open(Split::parse(&text).body))
    });
    Ok(notes)
}

/// Whether a heading outside code holds the word `open` or `waiting`.
fn left_open(body: &str) -> bool {
    let mut code = None;
    body.lines().any(|line| {
        match (code, fence(line)) {
            (None, Some(run)) => code = Some(run),
            // Only a run of the same mark, at least as long and alone on its line, closes a block.
            (Some(open), Some(run)) if run.starts_with(open) && line.trim() == run => code = None,
            (Some(_), _) => {}
            (None, None) => {
                return line.starts_with('#')
                    && line.trim_start_matches('#').starts_with(' ')
                    && line.split(|c: char| !c.is_alphanumeric()).any(|word| {
                        ["open", "waiting"]
                            .iter()
                            .any(|w| word.eq_ignore_ascii_case(w))
                    });
            }
        }
        false
    })
}

/// The run of backticks or tildes with which `line` opens or closes a code block.
fn fence(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let mark = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let after = line.trim_start_matches(mark);
    let run = &line[..line.len() - after.len()];
    // Three backticks with another later on the line are code inside a sentence.
    (run.len() >= 3 && !(mark == '`' && after.contains('`'))).then_some(run)
}
