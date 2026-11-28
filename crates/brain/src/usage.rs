//! Counting brain's own use: kinds of event and the door they came through, per day, in the
//! activity store, and each closed day's counts sent to omit, signed and ledgered. Nothing here
//! takes text. A count is never the reason an action fails.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use brain_index::activity::{self, Activity};
use brain_index::{Found, Index, is_gap};
use brain_note::Place;
use brain_platform::clock::Clock;
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Lock, Privacy, create_new};
use brain_report::{Count, Door, Event, Report, sign};
use ed25519_dalek::SigningKey;
use jiff::ToSpan;
use jiff::civil::Date;

use crate::config::Config;
use crate::gaps;
use crate::out::terminal;

/// The install's signing key, in the data folder; its public half names the install's reports.
const KEY: &str = "usage.key";
const SEND_LOCK: &str = "usage.lock";
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// How far back a run sends days it missed.
const CATCH_UP_DAYS: i64 = 7;

/// The activity store beside the index.
pub fn store(dirs: &Dirs) -> PathBuf {
    dirs.data.join("activity.db")
}

/// Runs `record` against the store; what cannot be saved prints one line and is dropped.
fn record(store: &Path, record: impl FnOnce(&Activity) -> anyhow::Result<()>) {
    if let Err(e) = Activity::open(store).and_then(|a| record(&a)) {
        eprintln!(
            "{}",
            terminal(&format!("brain: a usage count was not saved: {e:#}"))
        );
    }
}

/// Adds one to `event` today.
pub fn count(store: &Path, today: Date, event: Event, door: Option<Door>) {
    record(store, |a| a.count(today, event, door));
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
    record(store, |a| {
        a.count(today, Event::Question, Some(door))?;
        a.count(today, latency(ms), Some(door))?;
        if is_gap(found) {
            let area = gaps::area(&found.hits);
            a.log_gap(clock.now(), door, query, area.as_deref())?;
            a.count(today, Event::GapLogged, Some(door))
        } else if found
            .hits
            .iter()
            .any(|h| h.place == Place::Kept && !h.added)
        {
            a.count(today, Event::AnswerKept, Some(door))
        } else {
            Ok(())
        }
    });
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
    let activity = Activity::open(&store(dirs))?;
    for (event, door, n) in activity.day(today)? {
        lines.push(format!(
            "{}  {}  {n}",
            event.as_str(),
            door.map_or("-", Door::as_str)
        ));
    }
    let sends = activity.sends()?;
    if !sends.is_empty() {
        lines.push("Sent, each exactly as it left:".into());
    }
    for s in sends {
        lines.push(format!("{}  {}  {}", s.day, s.at, s.outcome));
        lines.push(String::from_utf8_lossy(&s.body).into_owned());
        lines.push(format!("signature {}", s.signature));
    }
    println!("{}", terminal(&lines.join("\n")));
    Ok(())
}

/// What one run of `send_due` did.
#[derive(Debug, Default)]
pub struct Sent {
    pub sent: usize,
    pub refused: usize,
    /// The outcome that stopped the run, when a send failed.
    pub failed: Option<String>,
    /// Another process held the send lock, so this run sent nothing.
    pub busy: bool,
}

impl Sent {
    pub fn line(&self) -> String {
        if self.busy {
            return "usage: another brain is sending, so this run sent nothing".into();
        }
        let mut line = format!("usage: {} sent, {} refused", self.sent, self.refused);
        if let Some(f) = &self.failed {
            line.push_str(&format!(", stopped at {f}"));
        }
        line
    }
}

/// Sends every closed day of the last week not yet settled, oldest first. Each day's report is
/// ledgered before it is sent and only the ledgered bytes are ever sent, so a resend repeats them
/// exactly. A day sent or refused is settled; a failure stops the run until the next one.
pub fn send_due(dirs: &Dirs, index: &Index, clock: &dyn Clock) -> anyhow::Result<Sent> {
    let mut out = Sent::default();
    // One sender at a time, so the daemon and `brain usage send` never send a day twice.
    let Some(_lock) = Lock::try_acquire(&dirs.data.join(SEND_LOCK))? else {
        out.busy = true;
        return Ok(out);
    };
    // Another vault's due notes are not counted.
    let current = index
        .check(&Config::load(dirs)?.vault)
        .is_ok()
        .then_some(index);
    let activity = Activity::open(&store(dirs))?;
    let today = clock.today();
    // Nothing counted yet leaves nothing to send: the window starts today.
    let mut day =
        (activity.send_from()?.unwrap_or(today)).max(today.checked_sub(CATCH_UP_DAYS.days())?);
    let client = reqwest::blocking::Client::builder()
        .timeout(SEND_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut key = None;
    loop {
        // Read before every day, so `brain usage off` stops even a run already going, and a run
        // that finds sending off keeps today's counts and every earlier one here for good.
        let config = Config::load(dirs)?;
        if !config.usage.send {
            activity.off(today)?;
            break;
        }
        if day >= today {
            break;
        }
        let row = match activity.send(day)? {
            Some(row) => row,
            None => {
                let key = match &key {
                    Some(k) => k,
                    None => key.insert(install_key(dirs)?),
                };
                let (body, signature) = report(&activity, current, &config.me, key, day)?;
                activity.ledger(day, clock.now(), &body, &signature)?;
                activity
                    .send(day)?
                    .context("the usage ledger lost the row just written")?
            }
        };
        if row.outcome == "pending" || row.outcome.starts_with("failed") {
            let outcome = post(&client, &config.usage.endpoint, &row);
            activity.outcome(day, &outcome)?;
            if outcome == "sent" {
                out.sent += 1;
            } else if outcome.starts_with("refused") {
                out.refused += 1;
            } else {
                count(&store(dirs), today, Event::ErrorSend, None);
                out.failed = Some(outcome);
                break;
            }
        }
        day = day.tomorrow()?;
    }
    Ok(out)
}

/// `day`'s report: its counts, whether anyone was active, and the notes due by then.
fn report(
    activity: &Activity,
    index: Option<&Index>,
    me: &str,
    key: &SigningKey,
    day: Date,
) -> anyhow::Result<(Vec<u8>, String)> {
    let mut counts: Vec<Count> = activity
        .day(day)?
        .into_iter()
        .map(|(event, door, n)| Count { event, door, n })
        .collect();
    // brain's own failures are not someone using it.
    let people = u32::from(counts.iter().any(|c| {
        !matches!(
            c.event,
            Event::ErrorIndex
                | Event::ErrorEmbed
                | Event::ErrorWatch
                | Event::ErrorSend
                | Event::ErrorUpdate
        )
    }));
    let due = match index {
        Some(index) => index.due(me, day)?.len() as u64,
        None => 0,
    };
    if due > 0 {
        counts.push(Count {
            event: Event::NotesDue,
            door: None,
            n: due,
        });
    }
    let report = Report::new(key, day, env!("CARGO_PKG_VERSION"), people, counts);
    Ok(sign(&report, key))
}

/// The outcome the ledger records: `sent`, `refused: <status>` for a body the endpoint will never
/// take, or `failed: <status or error kind>` for one worth sending again.
fn post(client: &reqwest::blocking::Client, endpoint: &str, row: &activity::Send) -> String {
    let sent = client
        .post(endpoint)
        .header("brain-signature", &row.signature)
        .header("content-type", "application/json")
        .body(row.body.clone())
        .send();
    match sent {
        Ok(r) => {
            let status = r.status();
            if status.is_success() {
                "sent".into()
            } else if matches!(status.as_u16(), 400 | 401 | 413 | 422) {
                // Only the endpoint's own refusals settle a day; a 403 or 404 from a proxy or a
                // wrong path says nothing about the report.
                format!("refused: {}", status.as_u16())
            } else {
                format!("failed: {}", status.as_u16())
            }
        }
        Err(e) => format!("failed: {}", error_kind(&e)),
    }
}

fn error_kind(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "TimedOut".into();
    }
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        if let Some(io) = s.downcast_ref::<std::io::Error>() {
            return format!("{:?}", io.kind());
        }
        source = s.source();
    }
    if e.is_connect() { "Connect" } else { "Request" }.into()
}

/// The install's signing key, made by the first process that needs it; a process that loses the
/// race to make it reads the winner's.
fn install_key(dirs: &Dirs) -> anyhow::Result<SigningKey> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no randomness for a key: {e}"))?;
    match create_new(&dirs.data, Path::new(KEY), &bytes, Privacy::Private) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            let path = dirs.data.join(KEY);
            bytes = std::fs::read(&path)?.try_into().map_err(|_| {
                anyhow::anyhow!(
                    "{} is not a 32-byte key; delete it and brain makes a new one",
                    path.display()
                )
            })?;
        }
        Err(e) => return Err(e).context("writing the usage key"),
    }
    Ok(SigningKey::from_bytes(&bytes))
}

/// Marks a day that had sending off for part of it, so it is never sent.
pub fn mark_switched(dirs: &Dirs, today: Date) -> anyhow::Result<()> {
    Activity::open(&store(dirs))?.off(today)
}

/// `brain usage on|off`. Either way today had sending off for part of it, so it is never sent.
pub fn set(dirs: &Dirs, clock: &dyn Clock, send: bool) -> anyhow::Result<()> {
    Config::edit(dirs, |config| config.usage.send = send)?;
    mark_switched(dirs, clock.today())?;
    println!("Usage sending: {}", if send { "on" } else { "off" });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_buckets_split_at_100_300_and_1000() {
        let cases = [
            (0, Event::SearchUnder100ms),
            (99, Event::SearchUnder100ms),
            (100, Event::SearchUnder300ms),
            (299, Event::SearchUnder300ms),
            (300, Event::SearchUnder1s),
            (999, Event::SearchUnder1s),
            (1000, Event::SearchOver1s),
        ];
        for (ms, event) in cases {
            assert_eq!(latency(ms), event, "{ms} ms");
        }
    }
}
