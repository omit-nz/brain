//! `verdicts.db`: one row per verdict, with what it picked.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use brain_index::db;
use jiff::{SignedDuration, Timestamp};
use rusqlite::{Connection, params};

const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_verdicts.sql")];

/// A hook has 250 ms in all, so it never waits long for another writer.
const BUSY: Duration = Duration::from_millis(50);
/// What the daemon waits, whose removals are not tried again.
const PATIENT: Duration = Duration::from_secs(5);

/// How long a verdict nobody labelled is kept.
const RETENTION: SignedDuration = SignedDuration::from_hours(90 * 24);

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub reflex: String,
    pub version: u32,
    pub backend: String,
    pub point: String,
    pub at: Timestamp,
    pub session: Option<String>,
    /// The trace run that holds how it was decided.
    pub run: Option<String>,
    pub input_text: String,
    pub fired: bool,
    pub confidence: f32,
    pub reason: String,
    pub acted: bool,
    pub latency_ms: i64,
    pub picks: Vec<PickRow>,
    /// Whose text this row holds (`note:<id>`, `session:<id>`, `file:<path>`), sorted.
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickRow {
    pub key: String,
    pub score: f32,
    pub text: String,
    /// False when it was under the floor or the session had been shown this key.
    pub passed: bool,
}

pub struct Verdicts {
    conn: Connection,
}

impl Verdicts {
    pub fn open(path: &Path) -> anyhow::Result<Verdicts> {
        let conn = db::open(path, MIGRATIONS)?;
        conn.busy_timeout(BUSY)?;
        Ok(Verdicts { conn })
    }

    /// For a caller with no hook's budget to keep.
    pub(crate) fn wait_longer(&self) -> anyhow::Result<()> {
        Ok(self.conn.busy_timeout(PATIENT)?)
    }

    pub fn insert(&mut self, row: &Row) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO verdicts(reflex, version, point, at, session, run, input_text, fired,
                                  confidence, reason, acted, latency_ms, backend)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                row.reflex,
                row.version,
                row.point,
                row.at.as_second(),
                row.session,
                row.run,
                row.input_text,
                row.fired,
                row.confidence,
                row.reason,
                row.acted,
                row.latency_ms,
                row.backend
            ],
        )?;
        let id = tx.last_insert_rowid();
        for (rank, p) in row.picks.iter().enumerate() {
            tx.execute(
                "INSERT INTO picks(verdict_id, rank, key, score, text, passed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, rank as i64, p.key, p.score, p.text, p.passed],
            )?;
        }
        for source in &row.sources {
            tx.execute(
                "INSERT OR IGNORE INTO verdict_sources(verdict_id, source) VALUES (?1, ?2)",
                params![id, source],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every verdict, oldest first.
    pub fn rows(&self) -> anyhow::Result<Vec<Row>> {
        let mut st = self.conn.prepare(
            "SELECT id, reflex, version, point, at, session, run, input_text, fired, confidence,
                    reason, acted, latency_ms, backend
             FROM verdicts ORDER BY id",
        )?;
        let mut picks = self.conn.prepare(
            "SELECT key, score, text, passed FROM picks WHERE verdict_id = ?1 ORDER BY rank",
        )?;
        let mut sources = self
            .conn
            .prepare("SELECT source FROM verdict_sources WHERE verdict_id = ?1 ORDER BY source")?;
        let mut rows = st.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            out.push(Row {
                reflex: r.get(1)?,
                version: r.get(2)?,
                point: r.get(3)?,
                at: Timestamp::from_second(r.get(4)?)?,
                session: r.get(5)?,
                run: r.get(6)?,
                input_text: r.get(7)?,
                fired: r.get(8)?,
                confidence: r.get(9)?,
                reason: r.get(10)?,
                acted: r.get(11)?,
                latency_ms: r.get(12)?,
                backend: r.get(13)?,
                sources: sources
                    .query_map([id], |s| s.get(0))?
                    .collect::<Result<_, _>>()?,
                picks: picks
                    .query_map([id], |p| {
                        Ok(PickRow {
                            key: p.get(0)?,
                            score: p.get(1)?,
                            text: p.get(2)?,
                            passed: p.get(3)?,
                        })
                    })?
                    .collect::<Result<_, _>>()?,
            });
        }
        Ok(out)
    }

    /// The keys `backend` of `reflex` has passed in `session`.
    pub fn passed_keys(
        &self,
        reflex: &str,
        backend: &str,
        session: &str,
    ) -> anyhow::Result<HashSet<String>> {
        let mut st = self.conn.prepare(
            "SELECT picks.key FROM picks JOIN verdicts ON verdicts.id = picks.verdict_id
             WHERE verdicts.reflex = ?1 AND verdicts.backend = ?2 AND verdicts.session = ?3
                 AND picks.passed",
        )?;
        let keys = st.query_map([reflex, backend, session], |r| r.get(0))?;
        Ok(keys.collect::<Result<_, _>>()?)
    }

    /// Removes every verdict that holds `source`'s text, labelled or not.
    pub fn purge(&self, source: &str) -> anyhow::Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM verdicts WHERE id IN
                 (SELECT verdict_id FROM verdict_sources WHERE source = ?1)",
            [source],
        )?)
    }

    /// Removes the verdicts nobody labelled that are more than 90 days older than `now`.
    pub fn prune(&self, now: Timestamp) -> anyhow::Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM verdicts WHERE at < ?1 AND label IS NULL AND id NOT IN
                 (SELECT verdict_id FROM picks WHERE label IS NOT NULL)",
            [(now - RETENTION).as_second()],
        )?)
    }
}
