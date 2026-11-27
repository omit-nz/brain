//! Brings the index in line with the vault. A note whose size and mtime are unchanged is not read;
//! one whose bytes hash the same only has its stat refreshed. Writes go in batches of 200, one
//! transaction each, so a killed run keeps its whole batches and the next run finishes the rest.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use brain_note::{Entry, Note, Place, Skip, Vault, ensure_id, reassign_id, rel_key};
use rusqlite::{OptionalExtension, Transaction, params};

use crate::Index;

const BATCH: usize = 200;
/// The mtime stored for a note indexed before its id was written. No file has it (`stat` gives
/// none below 0), so the next pass always reads the note again.
const UNSETTLED: i64 = -1;

#[derive(Debug, Default)]
pub struct Reindexed {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    pub skipped: Vec<(String, Skip)>,
    pub ids_written: usize,
    pub ids_reassigned: usize,
    /// Notes indexed with no id, because none can be written into their frontmatter.
    pub unidentified: Vec<String>,
    /// Notes indexed with their id left for a later pass: they need one but were saved too
    /// recently to write it.
    pub deferred: usize,
    /// Ids no note holds any more, and ids whose note stopped being shared: whatever was kept of
    /// their text elsewhere should go.
    pub gone: Vec<uuid::Uuid>,
}

struct Row {
    size: i64,
    mtime_ns: i64,
    hash: Vec<u8>,
    uuid: Option<String>,
}

/// A note this run reads and writes.
struct Work {
    rel: PathBuf,
    key: String,
    place: Place,
    raw: String,
    size: i64,
    mtime_ns: i64,
    note: Note,
    uuid: Option<String>,
}

fn stat(path: &Path) -> std::io::Result<(i64, i64)> {
    let m = fs::metadata(path)?;
    let mtime = m
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i64);
    Ok((m.len() as i64, mtime))
}

impl Work {
    /// Reads the note again after its id was written into it.
    fn reload(&mut self, vault: &Vault) -> anyhow::Result<()> {
        self.raw = vault
            .read(&self.rel)
            .map_err(|why| anyhow::anyhow!("{}: {}", self.key, why.describe()))?;
        (self.size, self.mtime_ns) = stat(&vault.root.join(&self.rel))?;
        self.note = Note::parse(&self.rel, &self.raw, self.place);
        Ok(())
    }
}

/// A test that kills a reindex mid-batch sets this; release builds never read it.
fn abort_after() -> Option<usize> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("BRAIN_TEST_ABORT_AFTER_NOTES")
        .ok()?
        .parse()
        .ok()
}

impl Index {
    pub fn reindex(&mut self, vault: &Vault) -> anyhow::Result<Reindexed> {
        self.reindex_with(vault, None)
    }

    /// As `reindex`, but a note saved after `settled_before` (and not after `now`) that still
    /// needs an id is left for a later pass, so an id is never written into a note someone is
    /// typing. A note stamped later than `now` came from another clock, and gets its id.
    pub fn reindex_settled(
        &mut self,
        vault: &Vault,
        settled_before: jiff::Timestamp,
        now: jiff::Timestamp,
    ) -> anyhow::Result<Reindexed> {
        self.reindex_with(vault, Some(settled_before..=now))
    }

    fn reindex_with(
        &mut self,
        vault: &Vault,
        typing: Option<std::ops::RangeInclusive<jiff::Timestamp>>,
    ) -> anyhow::Result<Reindexed> {
        // A vault folder that is gone would read as a vault with no notes and empty the index.
        anyhow::ensure!(
            vault.root.is_dir(),
            "the vault folder {} is not there; the index is left as it was",
            vault.root.display()
        );
        let abort_after = abort_after();
        let existing = self.rows()?;
        let held = self.held()?;
        // Rows written under other reading rules are re-derived, changed or not.
        let rules: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'rules'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let reread = rules != Some(brain_note::RULES.to_string());
        let mut out = Reindexed::default();
        let mut seen = HashSet::new();
        let mut work = Vec::new();

        for entry in vault.walk() {
            let (rel, place) = match entry {
                Entry::Skipped { rel, why } => {
                    // Notes under a folder that failed to read keep their rows, as an unreadable
                    // note does.
                    if matches!(why, Skip::Unreadable(_)) {
                        let key = rel_key(&rel);
                        let folder = format!("{key}/");
                        seen.extend(
                            existing
                                .keys()
                                .filter(|k| **k == key || k.starts_with(&folder))
                                .cloned(),
                        );
                    }
                    out.skipped.push((rel_key(&rel), why));
                    continue;
                }
                Entry::Note { rel, place } => (rel, place),
            };
            let key = rel_key(&rel);
            // A note that only failed to read this time keeps its row: a locked or briefly
            // unreadable file is not a deleted one.
            let (size, mtime_ns) = match stat(&vault.root.join(&rel)) {
                Ok(s) => s,
                Err(e) => {
                    if existing.contains_key(&key) {
                        seen.insert(key.clone());
                    }
                    out.skipped.push((key, Skip::Unreadable(e.to_string())));
                    continue;
                }
            };
            if !reread
                && existing
                    .get(&key)
                    .is_some_and(|r| r.size == size && r.mtime_ns == mtime_ns)
            {
                seen.insert(key);
                continue;
            }
            match vault.read(&rel) {
                Err(why) => {
                    if matches!(why, Skip::Unreadable(_)) && existing.contains_key(&key) {
                        seen.insert(key.clone());
                    }
                    out.skipped.push((key, why));
                }
                Ok(raw) => {
                    seen.insert(key.clone());
                    let note = Note::parse(&rel, &raw, place);
                    work.push(Work {
                        rel,
                        key,
                        place,
                        raw,
                        size,
                        mtime_ns,
                        note,
                        uuid: None,
                    });
                }
            }
        }

        // Who keeps each id: notes this run leaves alone, then notes that held their id in the
        // index before. Everyone else meets the rest in walk order, and a copy gets a new id.
        let mut claimed: HashMap<String, String> = HashMap::new();
        let in_work: HashSet<&str> = work.iter().map(|w| w.key.as_str()).collect();
        for (key, row) in &existing {
            if let Some(u) = &row.uuid
                && seen.contains(key)
                && !in_work.contains(key.as_str())
            {
                claimed.insert(u.clone(), key.clone());
            }
        }
        for w in &work {
            if let Some(u) = w.note.id.map(|u| u.to_string())
                && existing.get(&w.key).and_then(|r| r.uuid.as_ref()) == Some(&u)
            {
                claimed.insert(u, w.key.clone());
            }
        }
        // A note whose id cannot be written (broken frontmatter, a read-only folder, a file held
        // open) is indexed without one: searchable by its owner, never synced. So is a note still
        // being typed, until the first pass after it settles writes its id.
        for w in &mut work {
            if w.note.id.is_none()
                && typing.as_ref().is_some_and(|t| {
                    t.contains(
                        &jiff::Timestamp::from_nanosecond(w.mtime_ns.into()).unwrap_or_default(),
                    )
                })
            {
                out.deferred += 1;
                w.mtime_ns = UNSETTLED;
                continue;
            }
            let root = &vault.root;
            let (wrote, keeps_id) = match w.note.id.map(|u| u.to_string()) {
                Some(u) if claimed.get(&u).is_some_and(|holder| *holder != w.key) => {
                    match reassign_id(root, &w.rel, &w.raw) {
                        Ok(_) => {
                            out.ids_reassigned += 1;
                            (true, true)
                        }
                        Err(_) => (false, false),
                    }
                }
                Some(u) => {
                    claimed.insert(u, w.key.clone());
                    (false, true)
                }
                None => match ensure_id(root, &w.rel, &w.raw) {
                    Ok((_, wrote)) => {
                        out.ids_written += usize::from(wrote);
                        (wrote, true)
                    }
                    Err(_) => (false, false),
                },
            };
            if wrote {
                w.reload(vault)?;
                if let Some(u) = w.note.id {
                    claimed.insert(u.to_string(), w.key.clone());
                }
            }
            w.uuid = w.note.id.filter(|_| keeps_id).map(|u| u.to_string());
        }
        let tx = self.conn.transaction()?;
        for key in existing.keys().filter(|k| !seen.contains(*k)) {
            tx.execute("DELETE FROM notes WHERE path = ?1", [key])?;
            out.removed += 1;
        }
        tx.commit()?;

        let mut written = 0;
        for chunk in work.chunks(BATCH) {
            let tx = self.conn.transaction()?;
            for w in chunk {
                match existing.get(&w.key) {
                    None => {
                        upsert(&tx, w)?;
                        out.added += 1;
                    }
                    Some(row) if !reread && row.hash == w.note.hash && row.uuid == w.uuid => {
                        tx.execute(
                            "UPDATE notes SET size = ?1, mtime_ns = ?2 WHERE path = ?3",
                            params![w.size, w.mtime_ns, w.key],
                        )?;
                    }
                    Some(_) => {
                        upsert(&tx, w)?;
                        out.changed += 1;
                    }
                }
                written += 1;
                if abort_after == Some(written) {
                    std::process::abort();
                }
            }
            tx.commit()?;
        }

        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM skipped WHERE path NOT IN (SELECT path FROM notes WHERE embedded = -1)",
            [],
        )?;
        for (path, why) in &out.skipped {
            tx.execute(
                "INSERT INTO skipped(path, why) VALUES (?1, ?2)
                 ON CONFLICT(path) DO UPDATE SET why = excluded.why",
                params![path, why.describe()],
            )?;
        }
        tx.execute(
            "INSERT INTO meta(key, value) VALUES ('rules', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [brain_note::RULES.to_string()],
        )?;
        tx.commit()?;

        let mut st = self
            .conn
            .prepare("SELECT path FROM notes WHERE note_uuid IS NULL ORDER BY path")?;
        out.unidentified = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        drop(st);
        let now = self.held()?;
        out.gone = held
            .into_iter()
            .filter(|(id, was)| match now.get(id) {
                None => true,
                Some(is) => was == "shared" && is != "shared",
            })
            .filter_map(|(id, _)| id.parse().ok())
            .collect();
        Ok(out)
    }

    /// Each id in the index with its note's disclosure.
    fn held(&self) -> anyhow::Result<HashMap<String, String>> {
        let mut st = self
            .conn
            .prepare("SELECT note_uuid, disclosure FROM notes WHERE note_uuid IS NOT NULL")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn rows(&self) -> anyhow::Result<HashMap<String, Row>> {
        let mut st = self
            .conn
            .prepare("SELECT path, size, mtime_ns, hash, note_uuid FROM notes")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get(0)?,
                Row {
                    size: r.get(1)?,
                    mtime_ns: r.get(2)?,
                    hash: r.get(3)?,
                    uuid: r.get(4)?,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn upsert(tx: &Transaction<'_>, w: &Work) -> anyhow::Result<()> {
    let n = &w.note;
    // A note whose id this run moved to it from a note rewritten later in the run: free the id
    // first, and that note takes its own new id when its turn comes.
    if let Some(u) = &w.uuid {
        tx.execute(
            "UPDATE notes SET note_uuid = NULL WHERE note_uuid = ?1 AND path != ?2",
            params![u, w.key],
        )?;
    }
    let id: i64 = tx.query_row(
        "INSERT INTO notes(note_uuid, path, hash, size, mtime_ns, title, tags, kind, place,
                           disclosure, owner, review, embedded)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0)
         ON CONFLICT(path) DO UPDATE SET
             note_uuid = excluded.note_uuid, hash = excluded.hash, size = excluded.size,
             mtime_ns = excluded.mtime_ns, title = excluded.title, tags = excluded.tags,
             kind = excluded.kind, place = excluded.place, disclosure = excluded.disclosure,
             owner = excluded.owner, review = excluded.review
         RETURNING id",
        params![
            w.uuid,
            w.key,
            &n.hash[..],
            w.size,
            w.mtime_ns,
            n.title,
            serde_json::to_string(&n.tags)?,
            n.kind.as_str(),
            w.place.as_str(),
            n.disclosure.as_str(),
            n.owner,
            n.review.map(|d| d.to_string()),
        ],
        |r| r.get(0),
    )?;
    // A vector is made from these three, so a note rewritten with the same text keeps it: a
    // reread under new rules, or an edit to its other frontmatter, embeds nothing again.
    let rewritten = tx.execute(
        "INSERT INTO notes_content(id, title, tags, body) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET title = excluded.title, tags = excluded.tags,
                                       body = excluded.body
         WHERE (title, tags, body) IS NOT (excluded.title, excluded.tags, excluded.body)",
        params![id, n.title, n.tags.join(" "), n.body],
    )?;
    if rewritten > 0 {
        tx.execute("DELETE FROM embeddings WHERE id = ?1", [id])?;
    }
    // A note with no vector is pending: a new one, a rewritten one, and one the embedder failed
    // on, which is tried again.
    tx.execute(
        "UPDATE notes SET embedded = 0
         WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM embeddings WHERE id = ?1)",
        [id],
    )?;
    tx.execute("DELETE FROM links WHERE source_id = ?1", [id])?;
    for target in &n.links {
        tx.execute(
            "INSERT OR IGNORE INTO links(source_id, target) VALUES (?1, ?2)",
            params![id, target],
        )?;
    }
    Ok(())
}
