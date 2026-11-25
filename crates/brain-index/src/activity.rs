//! The activity store, `activity.db` beside the index: counts of what brain did, per day, kind
//! and door, and the gaps (questions no note answered). It is not a cache of the vault, so it
//! is kept when the index is rebuilt. Counts hold names and numbers only; a gap's question stays
//! in `gaps`, which no report reads.

use std::path::Path;

use brain_report::{Door, Event};
use jiff::Timestamp;
use jiff::civil::Date;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db;

const MIGRATIONS: &[&str] = &[include_str!("../migrations/activity/0001_activity.sql")];

pub struct Activity {
    conn: Connection,
}

/// A question no note answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Gap {
    pub id: i64,
    pub at: Timestamp,
    pub door: Door,
    pub query: String,
    pub area: Option<String>,
}

impl Activity {
    pub fn open(path: &Path) -> anyhow::Result<Activity> {
        Ok(Activity {
            conn: db::open(path, MIGRATIONS)?,
        })
    }

    /// Adds one to `event` on `day` through `door`, and remembers the first day anything was
    /// counted.
    pub fn count(&self, day: Date, event: Event, door: Option<Door>) -> anyhow::Result<()> {
        let day = day.to_string();
        self.conn.execute(
            "INSERT INTO counts(day, kind, door, n) VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(day, kind, door) DO UPDATE SET n = n + 1",
            params![day, event.as_str(), door.map_or("", Door::as_str)],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('usage_first_day', ?1)",
            [&day],
        )?;
        Ok(())
    }

    /// Every count of `day`. A row whose kind or door this build does not know is left out.
    pub fn day(&self, day: Date) -> anyhow::Result<Vec<(Event, Option<Door>, u64)>> {
        let mut st = self
            .conn
            .prepare("SELECT kind, door, n FROM counts WHERE day = ?1 ORDER BY kind, door")?;
        let rows = st.query_map([day.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, door, n) = row?;
            let door = match door.as_str() {
                "" => None,
                d => match Door::parse(d) {
                    Some(d) => Some(d),
                    None => continue,
                },
            };
            if let Some(event) = Event::parse(&kind) {
                out.push((event, door, n as u64));
            }
        }
        Ok(out)
    }

    pub fn first_day(&self) -> anyhow::Result<Option<Date>> {
        let day: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'usage_first_day'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(day.map(|d| d.parse()).transpose()?)
    }

    pub fn log_gap(
        &self,
        at: Timestamp,
        door: Door,
        query: &str,
        area: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO gaps(at, door, query, area) VALUES (?1, ?2, ?3, ?4)",
            params![at.to_string(), door.as_str(), query, area],
        )?;
        Ok(())
    }

    /// Open gaps, newest first.
    pub fn open_gaps(&self, limit: usize) -> anyhow::Result<Vec<Gap>> {
        let mut st = self.conn.prepare(
            "SELECT id, at, door, query, area FROM gaps WHERE filled_at IS NULL
             ORDER BY at DESC, id DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, at, door, query, area) = row?;
            out.push(Gap {
                id,
                at: at.parse()?,
                door: Door::parse(&door).unwrap_or(Door::Other),
                query,
                area,
            });
        }
        Ok(out)
    }

    /// Marks a gap filled at `at`; a gap already filled keeps its first time.
    pub fn fill_gap(&self, id: i64, at: Timestamp) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE gaps SET filled_at = ?2 WHERE id = ?1 AND filled_at IS NULL",
            params![id, at.to_string()],
        )?;
        Ok(())
    }

    /// How many gaps were filled at or after `since`.
    pub fn filled_since(&self, since: Timestamp) -> anyhow::Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM gaps WHERE filled_at >= ?1",
            [since.to_string()],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
}
