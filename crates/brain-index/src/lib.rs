//! The index: one SQLite file of notes, their text and links, rebuilt from the vault alone. Beside
//! it, the activity store keeps what the index cannot rebuild: counts and gaps.

pub mod activity;
pub mod db;
pub mod embed;
mod recent;
mod reindex;
mod search;

use std::collections::BTreeMap;
use std::path::Path;

use brain_note::Kind;
use rusqlite::{Connection, OptionalExtension};

pub use embed::{Embedded, HashEmbedder};
pub use reindex::Reindexed;
pub use search::{Found, Hit, Query, RERANK_FLOOR, Scope, is_gap};

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> anyhow::Result<Index> {
        Ok(Index {
            conn: db::open(path, db::INDEX_MIGRATIONS)?,
        })
    }

    /// Refuses to answer for `vault` from another vault's notes. An index that names no vault yet,
    /// as one from an older release does, answers until its next reindex.
    pub fn check(&self, vault: &Path) -> anyhow::Result<()> {
        let held: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'vault'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let vault = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
        match held {
            Some(held) if Path::new(&held) != vault => anyhow::bail!(
                "brain's index still holds {held}; it is being rebuilt for {} (or run `brain index`)",
                vault.display()
            ),
            _ => Ok(()),
        }
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

    /// The indexed note `target` names, by id or by vault path; only notes the index holds,
    /// never another file in the vault.
    pub fn lookup(&self, target: &str) -> anyhow::Result<Option<Noted>> {
        let by = if uuid::Uuid::parse_str(target).is_ok() {
            "note_uuid"
        } else {
            "path"
        };
        Ok(self
            .conn
            .query_row(
                &format!("SELECT note_uuid, path, title FROM notes WHERE {by} = ?1"),
                [target],
                |r| {
                    Ok(Noted {
                        note_uuid: r.get::<_, Option<String>>(0)?.and_then(|u| u.parse().ok()),
                        path: r.get(1)?,
                        title: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// `owner`'s kept notes whose review falls on or before `by`, oldest review first.
    pub fn due(&self, owner: &str, by: jiff::civil::Date) -> anyhow::Result<Vec<Due>> {
        let mut st = self.conn.prepare(
            "SELECT note_uuid, path, title, review, kind FROM notes
             WHERE place = 'kept' AND owner = ?1 AND review <= ?2
             ORDER BY review, path",
        )?;
        let rows = st.query_map([owner, &by.to_string()], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (uuid, path, title, review, kind) = row?;
            out.push(Due {
                note_uuid: uuid.and_then(|u| u.parse().ok()),
                path,
                title,
                review: review.parse()?,
                kind: Kind::parse(&kind),
            });
        }
        Ok(out)
    }
}

/// An indexed note, named.
#[derive(Debug, Clone)]
pub struct Noted {
    pub note_uuid: Option<uuid::Uuid>,
    pub path: String,
    pub title: String,
}

/// A kept note whose review has come.
#[derive(Debug, Clone)]
pub struct Due {
    pub note_uuid: Option<uuid::Uuid>,
    pub path: String,
    pub title: String,
    pub review: jiff::civil::Date,
    pub kind: Kind,
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
