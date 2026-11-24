//! The index: one SQLite file of notes, their text and links, rebuilt from the vault alone.

pub mod db;
pub mod embed;
mod reindex;
mod search;

use std::path::Path;

use rusqlite::Connection;

pub use embed::{Embedded, HashEmbedder};
pub use reindex::Reindexed;
pub use search::{Hit, Query, Scope};

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> anyhow::Result<Index> {
        Ok(Index {
            conn: db::open(path)?,
        })
    }

    pub fn note_count(&self) -> anyhow::Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))?;
        Ok(n as usize)
    }
}
