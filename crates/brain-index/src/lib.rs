//! The index: one SQLite file of notes, their text and links, rebuilt from the vault alone. Beside
//! it, the activity store keeps what the index cannot rebuild: counts and gaps.

pub mod activity;
pub mod db;
pub mod embed;
mod reindex;
mod search;

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

pub use embed::{Embedded, HashEmbedder};
pub use reindex::Reindexed;
pub use search::{Found, GAP_FLOOR, Hit, Query, Scope, is_gap};

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> anyhow::Result<Index> {
        Ok(Index {
            conn: db::open(path, db::INDEX_MIGRATIONS)?,
        })
    }

    pub fn note_count(&self) -> anyhow::Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    pub fn stats(&self) -> anyhow::Result<Stats> {
        let (notes, embedded, failed, pending, unidentified): (i64, i64, i64, i64, i64) =
            self.conn.query_row(
                "SELECT count(*),
                        count(*) FILTER (WHERE embedded = 1),
                        count(*) FILTER (WHERE embedded = -1),
                        count(*) FILTER (WHERE embedded = 0),
                        count(*) FILTER (WHERE note_uuid IS NULL)
                 FROM notes",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
        let mut skipped = BTreeMap::new();
        let mut st = self.conn.prepare("SELECT why FROM skipped")?;
        for why in st.query_map([], |r| r.get::<_, String>(0))? {
            let why = why?;
            let reason = why.split(':').next().unwrap_or_default().to_string();
            *skipped.entry(reason).or_insert(0) += 1;
        }
        Ok(Stats {
            notes: notes as usize,
            embedded: embedded as usize,
            failed: failed as usize,
            pending: pending as usize,
            unidentified: unidentified as usize,
            skipped,
        })
    }

    /// The vault path of the note holding `id`, if it is indexed.
    pub fn path_of(&self, id: &uuid::Uuid) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT path FROM notes WHERE note_uuid = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?)
    }
}

/// What the index holds, for `brain status`.
#[derive(Debug)]
pub struct Stats {
    pub notes: usize,
    pub embedded: usize,
    pub failed: usize,
    pub pending: usize,
    pub unidentified: usize,
    /// Skipped files and failed embeddings, counted by reason.
    pub skipped: BTreeMap<String, usize>,
}
