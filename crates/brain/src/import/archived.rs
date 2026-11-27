//! What `brain memory follow` archived, so that it can be undone. A record leaves the ledger for
//! `follow-archive.json` when its copy is moved under the archive folder, and goes back when the
//! file does. Nothing that works on the ledger can see a record here, so nothing there can lose one.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use brain_index::Index;
use brain_note::{Note, Place, Vault, rel_key};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, archive_name, rename_within, write_atomic};
use serde::{Deserialize, Serialize};

use super::follow::{Change, Followed, copy_of, source_of};
use super::{Ledger, LedgerEntry, Source, hash};
use crate::daemon;

const FILE: &str = "follow-archive.json";

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    key: String,
    /// Where the copy was put, relative to the vault.
    path: PathBuf,
    /// The last pass did not find the file there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    gone: bool,
    // The ledger's record of the copy, as it goes back; the vault and followed folder it is of
    // are always known.
    id: Option<String>,
    src_hash: String,
    dst_hash: String,
    from: PathBuf,
    vault: PathBuf,
}

impl Record {
    /// The ledger's entry for this copy, as it goes back.
    fn entry(&self) -> LedgerEntry {
        LedgerEntry {
            id: self.id.clone(),
            src_hash: self.src_hash.clone(),
            dst_hash: self.dst_hash.clone(),
            from: Some(self.from.clone()),
            vault: Some(self.vault.clone()),
            missing: false,
        }
    }

    /// Whether `entry` records this very copy of this source.
    fn is(&self, entry: &LedgerEntry) -> bool {
        entry.id == self.id && entry.src_hash == self.src_hash && entry.dst_hash == self.dst_hash
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Archive(Vec<Record>);

/// The bytes of the note at `rel` when its id is `id`; `None` when there is no such note there.
/// An error is a file that could not be read, which says neither.
fn holds(vault: &Vault, rel: &Path, id: Option<&str>) -> std::io::Result<Option<Vec<u8>>> {
    let path = vault.root.join(rel);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() => return Ok(None),
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    }
    let bytes = std::fs::read(&path)?;
    let found = Note::parse(rel, &String::from_utf8_lossy(&bytes), Place::Kept).id;
    let same = found.is_some_and(|found| Some(found.to_string().as_str()) == id);
    Ok(same.then_some(bytes))
}

impl Archive {
    pub fn load(dirs: &Dirs) -> anyhow::Result<Archive> {
        let path = dirs.data.join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(Archive::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, dirs: &Dirs) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        write_atomic(
            &dirs.data,
            Path::new(FILE),
            text.as_bytes(),
            Privacy::Private,
        )?;
        Ok(())
    }

    /// Whether a record names the note `id`.
    pub fn names(&self, id: &str) -> bool {
        self.0.iter().any(|r| r.id.as_deref() == Some(id))
    }

    /// How many archived copies of `vault` are there to come back.
    pub fn waiting(&self, vault: &Vault) -> usize {
        self.0
            .iter()
            .filter(|r| r.vault == vault.root)
            .filter(|r| matches!(holds(vault, &r.path, r.id.as_deref()), Ok(Some(_))))
            .count()
    }

    /// The source hash of `key`'s archived copy in `vault`.
    pub(super) fn src_hash_of(&self, key: &str, vault: &Path) -> Option<&str> {
        self.0
            .iter()
            .find(|r| r.key == key && r.vault == vault)
            .map(|r| r.src_hash.as_str())
    }

    pub(super) fn name_for(&self, vault: &Vault, rel: &Path) -> PathBuf {
        archive_name(&vault.root, rel, &vault.layout.archive, |name| {
            self.0.iter().any(|r| r.path == name)
        })
    }

    pub(super) fn add(
        &mut self,
        key: &str,
        path: PathBuf,
        entry: &LedgerEntry,
        followed: &Path,
        vault: &Path,
    ) {
        self.0.push(Record {
            key: key.to_string(),
            path,
            gone: false,
            id: entry.id.clone(),
            src_hash: entry.src_hash.clone(),
            dst_hash: entry.dst_hash.clone(),
            from: followed.to_path_buf(),
            vault: vault.to_path_buf(),
        });
    }

    /// Brings the records and the ledger back into step with each other and with `vault`, after
    /// a pass cut short, a newer copy or the person's own hand. The only place a record is
    /// dropped. The ledger is saved first: a record moved to it and not yet dropped here is
    /// finished by the next pass, and the other order would lose it. Returns what could not be
    /// read, whose records are left as they are.
    pub(super) fn reconcile(
        &mut self,
        dirs: &Dirs,
        vault: &Vault,
        ledger: &mut Ledger,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let (mut in_ledger, mut in_archive) = (false, false);
        let (mut deleted, mut unread) = (Vec::new(), Vec::new());
        self.0.retain_mut(|r| {
            if r.vault != vault.root {
                return true;
            }
            let id = r.id.as_deref();
            let there = (
                holds(vault, &r.path, id),
                holds(vault, &copy_of(vault, &r.key), id),
            );
            let (archived, copied) = match there {
                (Ok(archived), Ok(copied)) => (archived, copied),
                (Err(e), _) | (_, Err(e)) => {
                    unread.push((r.key.clone(), format!("cannot be read: {e}")));
                    return true;
                }
            };
            let keep = match ledger.current(&r.key, &vault.root) {
                // Cut short after the move: the ledger has not let go of it, and nothing of it
                // is left at the copy's path.
                Some(l) if r.is(l) && archived.is_some() && copied.is_none() => {
                    ledger.0.remove(&r.key);
                    in_ledger = true;
                    return true;
                }
                // Cut short before the move, restored, or a newer copy has taken its place. The
                // archived file, if there, is left with its words, so nothing is purged.
                Some(_) => false,
                None => {
                    // While the archived file is still there, only the very bytes are it: a
                    // fresh copy written under the same id is not.
                    let back = copied.is_some_and(|b| archived.is_none() || hash(&b) == r.dst_hash);
                    if back {
                        ledger.0.insert(r.key.clone(), r.entry());
                        in_ledger = true;
                        false
                    } else if archived.is_some() {
                        in_archive |= std::mem::replace(&mut r.gone, false);
                        return true;
                    } else if !r.gone {
                        r.gone = true;
                        in_archive = true;
                        return true;
                    } else {
                        // Deleted by the person: a removed copy from here on.
                        deleted.extend(r.id.clone());
                        ledger.0.insert(r.key.clone(), r.entry());
                        in_ledger = true;
                        false
                    }
                }
            };
            in_archive |= !keep;
            keep
        });
        if !deleted.is_empty() {
            // A file the person moved into their notes is a note there: its traces stay. A
            // lookup that fails purges nothing, and the traces age out.
            let index = Index::open(&dirs.data.join("index.db"))?;
            let nowhere = |id: &String| {
                uuid::Uuid::parse_str(id).is_ok_and(|id| matches!(index.path_of(&id), Ok(None)))
            };
            daemon::purge(dirs, deleted.into_iter().filter(nowhere));
        }
        if in_ledger {
            ledger.save(dirs)?;
        }
        if in_archive {
            self.save(dirs)?;
        }
        Ok(unread)
    }

    /// Moves back the archived copy of each source that has returned as it was. Returns the
    /// sources still to be classified, and the id each returned and changed one is to be copied
    /// under: its archived copy stays where it is, with the old words.
    pub(super) fn restore(
        &self,
        vault: &Vault,
        followed: &Path,
        sources: Vec<Source>,
        ledger: &mut Ledger,
        written: &mut Vec<String>,
        out: &mut Followed,
    ) -> (Vec<Source>, HashMap<String, String>) {
        let mut returning = HashMap::new();
        let mut left = Vec::new();
        for source in sources {
            let record = self
                .0
                .iter()
                .find(|r| r.key == source.key && r.vault == vault.root && r.from == followed);
            let Some(r) = record.filter(|_| ledger.current(&source.key, &vault.root).is_none())
            else {
                left.push(source);
                continue;
            };
            // The person deleted the archived file, or it cannot be read: not copied again.
            let Ok(Some(archived)) = holds(vault, &r.path, r.id.as_deref()) else {
                continue;
            };
            if hash(source.raw.as_bytes()) != r.src_hash {
                returning.extend(r.id.clone().map(|id| (source.key.clone(), id)));
                left.push(source);
                continue;
            }
            let to = copy_of(vault, &source.key);
            // A file already at the copy's path is left, and classify says so.
            if let Err(e) = rename_within(&vault.root, &r.path, &to) {
                if e.kind() != ErrorKind::AlreadyExists {
                    out.skipped
                        .push((source.key.clone(), format!("not restored: {e}")));
                    continue;
                }
                left.push(source);
                continue;
            }
            ledger.0.insert(source.key.clone(), r.entry());
            // Only a copy as brain wrote it is settled: settling the person's edit would record
            // their words as brain's, and the next change would write over them.
            if hash(&archived) == r.dst_hash {
                written.push(source.key.clone());
            }
            out.restored.push(rel_key(&to));
            out.changes.push(Change {
                what: "restore",
                file: source_of(followed, &source.key),
                key: source.key.clone(),
                put: None,
            });
            left.push(source);
        }
        (left, returning)
    }
}

/// What `status` says of `n` archived copies.
pub fn archived_line(n: usize) -> String {
    match n {
        1 => "1 followed memory is gone from Claude Code and out of search; it comes back if its file does".into(),
        n => format!("{n} followed memories are gone from Claude Code and out of search; they come back if their files do"),
    }
}

/// The order of reconcile's two saves, shown at the filesystem: with one file impossible to write,
/// what was written of the other is what came first.
#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "memory/-proj/one.md";
    const ID: &str = "0195f2e0-0000-7000-8000-000000000001";

    /// Writes a file in `root`, making its folders.
    fn put(root: &Path, rel: &str, text: &str) {
        write_atomic(root, Path::new(rel), text.as_bytes(), Privacy::Private).unwrap();
    }

    /// A record whose copy is back at its path (row 4), or whose files are gone for the second
    /// look (row 7): both move it to the ledger.
    fn moving(back: bool) -> (tempfile::TempDir, Dirs, Vault, Archive) {
        let home = tempfile::tempdir().unwrap();
        let dirs = Dirs::from_home(home.path());
        put(home.path(), "vault/other.md", "x");
        let vault = Vault::open(&home.path().join("vault")).unwrap();
        let text = format!("---\nid: {ID}\n---\nwords\n");
        if back {
            put(&vault.root, "working-memory/-proj/one.md", &text);
        }
        let archive = Archive(vec![Record {
            key: KEY.into(),
            path: "_archive/working-memory/-proj/one.md".into(),
            gone: !back,
            id: Some(ID.into()),
            src_hash: hash(b"source"),
            dst_hash: hash(text.as_bytes()),
            from: home.path().join("projects"),
            vault: vault.root.clone(),
        }]);
        (home, dirs, vault, archive)
    }

    #[test]
    fn the_records_are_not_saved_when_the_ledger_cannot_be() {
        for back in [true, false] {
            let (_home, dirs, vault, mut archive) = moving(back);
            // A folder where the ledger goes.
            put(&dirs.data, "import-ledger.json/x", "x");
            let done = archive.reconcile(&dirs, &vault, &mut Ledger::default());
            assert!(done.is_err(), "back {back}");
            assert!(!dirs.data.join(FILE).exists(), "back {back}: saved first");
        }
    }

    #[test]
    fn the_ledger_is_saved_before_the_records() {
        for back in [true, false] {
            let (_home, dirs, vault, mut archive) = moving(back);
            put(&dirs.data, "follow-archive.json/x", "x");
            let done = archive.reconcile(&dirs, &vault, &mut Ledger::default());
            assert!(done.is_err(), "back {back}");
            let ledger = Ledger::load(&dirs).unwrap();
            assert!(ledger.current(KEY, &vault.root).is_some(), "back {back}");
        }
    }
}
