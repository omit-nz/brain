//! Brings the index in line with the vault. A note whose size and mtime are unchanged is not read;
//! one whose bytes hash the same only has its stat refreshed. Writes go in batches of 200, one
//! transaction each, so a killed run keeps its whole batches and the next run finishes the rest.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use brain_note::{
    Disclosure, Entry, Kind, Note, Place, Skip, Vault, ensure_id, reassign_id, rel_key,
};
use rusqlite::{Transaction, params};

use crate::Index;

const BATCH: usize = 200;

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

fn describe(why: &Skip) -> String {
    match why {
        Skip::Link => "a link, which brain never follows".into(),
        Skip::TooLarge(n) => format!(
            "{n} bytes, over the {} byte limit",
            brain_note::MAX_NOTE_BYTES
        ),
        Skip::NotUtf8 => "not UTF-8 text".into(),
        Skip::Unreadable(e) => e.clone(),
    }
}

fn kind_text(k: Kind) -> &'static str {
    match k {
        Kind::Knowledge => "knowledge",
        Kind::Rule => "rule",
        Kind::History => "history",
    }
}

fn place_text(p: Place) -> &'static str {
    match p {
        Place::Inbox => "inbox",
        Place::Private => "private",
        Place::Kept => "kept",
    }
}

fn disclosure_text(d: Disclosure) -> &'static str {
    match d {
        Disclosure::Shared => "shared",
        Disclosure::Listed => "listed",
        Disclosure::Private => "private",
    }
}

impl Work {
    /// Reads the note again after its id was written into it.
    fn reload(&mut self, vault: &Vault) -> anyhow::Result<()> {
        self.raw = vault
            .read(&self.rel)
            .map_err(|why| anyhow::anyhow!("{}: {}", self.key, describe(&why)))?;
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
        let abort_after = abort_after();
        let existing = self.rows()?;
        let mut out = Reindexed::default();
        let mut seen = HashSet::new();
        let mut work = Vec::new();

        for entry in vault.walk() {
            let (rel, place) = match entry {
                Entry::Skipped { rel, why } => {
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
            if existing
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
        // open) is indexed without one: searchable by its owner, never synced.
        for w in &mut work {
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
                    Some(row) if row.hash == w.note.hash && row.uuid == w.uuid => {
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
                params![path, describe(why)],
            )?;
        }
        tx.commit()?;

        let mut st = self
            .conn
            .prepare("SELECT path FROM notes WHERE note_uuid IS NULL ORDER BY path")?;
        out.unidentified = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        Ok(out)
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
             owner = excluded.owner, review = excluded.review, embedded = 0
         RETURNING id",
        params![
            w.uuid,
            w.key,
            &n.hash[..],
            w.size,
            w.mtime_ns,
            n.title,
            serde_json::to_string(&n.tags)?,
            kind_text(n.kind),
            place_text(w.place),
            disclosure_text(n.disclosure),
            n.owner,
            n.review.map(|d| d.to_string()),
        ],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO notes_content(id, title, tags, body) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET title = excluded.title, tags = excluded.tags,
                                       body = excluded.body",
        params![id, n.title, n.tags.join(" "), n.body],
    )?;
    tx.execute("DELETE FROM embeddings WHERE id = ?1", [id])?;
    tx.execute("DELETE FROM links WHERE source_id = ?1", [id])?;
    for target in &n.links {
        tx.execute(
            "INSERT OR IGNORE INTO links(source_id, target) VALUES (?1, ?2)",
            params![id, target],
        )?;
    }
    Ok(())
}
