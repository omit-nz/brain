//! `setup.json`: what brain keeps about offering setup. It only guards reminders, so a file that
//! cannot be read is set aside and started again, never a failure that stops anything else.

use std::io::{self, ErrorKind};
use std::path::Path;

use anyhow::Context;
use brain_platform::clock::Clock;
use brain_platform::fs::{
    Lock, Privacy, archive_name, create_dir_private, rename_within, write_atomic,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

pub const FILE: &str = "setup.json";

/// Held by whoever reads the state to change it, until it is saved.
pub const LOCK: &str = "setup.lock";

/// What an unreadable file is set aside as, with the day.
const ASIDE: &str = "setup-unreadable-";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct State {
    offered_at: Option<Timestamp>,
    snoozed_until: Option<Timestamp>,
    reminders_off: bool,
}

/// The state, the default one when there is no file; `InvalidData` for one that does not parse.
fn read(data: &Path) -> io::Result<State> {
    match std::fs::read_to_string(data.join(FILE)) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e),
    }
}

/// Reads, changes and saves the state under the lock the caller holds. `None` when the file could
/// not be read: it is set aside under a name with the day, and nothing else is written.
fn edit<T>(
    data: &Path,
    clock: &dyn Clock,
    _held: &Lock,
    change: impl FnOnce(&mut State) -> T,
) -> anyhow::Result<Option<T>> {
    let Ok(before) = read(data) else {
        let name = format!("{ASIDE}{}.json", clock.today());
        let to = archive_name(data, Path::new(&name), Path::new(""), |_| false);
        rename_within(data, Path::new(FILE), &to)
            .with_context(|| format!("setting aside {}", data.join(FILE).display()))?;
        return Ok(None);
    };
    let mut state = before.clone();
    let out = change(&mut state);
    if state != before {
        let text = serde_json::to_string_pretty(&state)?;
        write_atomic(data, Path::new(FILE), text.as_bytes(), Privacy::Private)?;
    }
    Ok(Some(out))
}

/// Sets an unreadable state aside, changing nothing in a readable one.
pub fn tidy(data: &Path, clock: &dyn Clock, held: &Lock) -> anyhow::Result<()> {
    edit(data, clock, held, |_| ())?;
    Ok(())
}

/// `brain setup reminders on|off`.
pub fn set_reminders(data: &Path, clock: &dyn Clock, on: bool) -> anyhow::Result<()> {
    create_dir_private(data)?;
    let held = Lock::acquire(&data.join(LOCK))?;
    edit(data, clock, &held, |state| state.reminders_off = !on)?.context(
        "brain could not read its setup reminders, so it set the file aside; run the command \
         again",
    )
}

/// The names of the files set aside as unreadable.
pub fn set_aside(data: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(data)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(ASIDE))
        .collect();
    names.sort();
    names
}
