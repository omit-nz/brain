//! Counting brain's own use: kinds of event and the door they came through, per day, in the
//! activity store. Nothing here takes text. A count is never the reason an action fails.

use std::path::{Path, PathBuf};

use brain_index::activity::Activity;
use brain_index::{Found, is_gap};
use brain_note::Place;
use brain_platform::clock::Clock;
use brain_platform::dirs::Dirs;
use brain_report::{Door, Event};
use jiff::civil::Date;

use crate::config::Config;
use crate::gaps;
use crate::out::terminal;

/// The activity store beside the index.
pub fn store(dirs: &Dirs) -> PathBuf {
    dirs.data.join("activity.db")
}

/// Adds one to `event` today; a count that cannot be saved prints one line and is dropped.
pub fn count(store: &Path, today: Date, event: Event, door: Option<Door>) {
    if let Err(e) = Activity::open(store).and_then(|a| a.count(today, event, door)) {
        eprintln!(
            "{}",
            terminal(&format!("brain: a usage count was not saved: {e:#}"))
        );
    }
}

/// A search's time as one of four buckets, so the count says how fast without saying when.
pub fn latency(ms: u128) -> Event {
    match ms {
        0..100 => Event::SearchUnder100ms,
        100..300 => Event::SearchUnder300ms,
        300..1000 => Event::SearchUnder1s,
        _ => Event::SearchOver1s,
    }
}

/// What every door records after a search: the question and its speed, and either the gap (its
/// question kept in the gap log, never in a count) or, when a kept note answered, the answer.
pub fn searched(store: &Path, clock: &dyn Clock, door: Door, query: &str, found: &Found, ms: u128) {
    let today = clock.today();
    count(store, today, Event::Question, Some(door));
    count(store, today, latency(ms), Some(door));
    if is_gap(found) {
        let area = gaps::area(&found.hits);
        if let Err(e) =
            Activity::open(store).and_then(|a| a.log_gap(clock.now(), door, query, area.as_deref()))
        {
            eprintln!(
                "{}",
                terminal(&format!("brain: the gap was not saved: {e:#}"))
            );
        }
        count(store, today, Event::GapLogged, Some(door));
    } else if found.hits.iter().any(|h| h.place == Place::Kept) {
        count(store, today, Event::AnswerKept, Some(door));
    }
}

/// `brain usage`: whether sending is on, then today's counts.
pub fn show(dirs: &Dirs, today: Date) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let mut lines = vec![if config.usage.send {
        format!("Usage sending: on, to {}", config.usage.endpoint)
    } else {
        "Usage sending: off".into()
    }];
    lines.push("Today so far:".into());
    for (event, door, n) in Activity::open(&store(dirs))?.day(today)? {
        lines.push(format!(
            "{}  {}  {n}",
            event.as_str(),
            door.map_or("-", Door::as_str)
        ));
    }
    println!("{}", terminal(&lines.join("\n")));
    Ok(())
}

/// `brain usage on|off`.
pub fn set(dirs: &Dirs, send: bool) -> anyhow::Result<()> {
    let mut config = Config::load(dirs)?;
    config.usage.send = send;
    config.save(dirs)?;
    println!("Usage sending: {}", if send { "on" } else { "off" });
    Ok(())
}
