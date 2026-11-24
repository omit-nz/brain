//! The index: one SQLite file of notes, their text and links, rebuilt from the vault alone.

pub mod db;
pub mod embed;
mod reindex;

use std::path::Path;

use rusqlite::Connection;

pub use embed::{Embedded, HashEmbedder};
pub use reindex::Reindexed;

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> anyhow::Result<Index> {
        Ok(Index {
            conn: db::open(path)?,
        })
    }

    pub fn note_count(&self) -> usize {
        self.conn
            .query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0))
            .map_or(0, |n| n as usize)
    }
}
