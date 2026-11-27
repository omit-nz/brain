//! What `brain memory follow` archived, so that it can be undone. A record leaves the ledger for
//! `follow-archive.json` when its copy is moved under the archive folder, and goes back when the
//! file does. Nothing that works on the ledger can see a record here, so nothing there can lose one.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
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
    /// The ledger's record of the copy, as it goes back.
    #[serde(flatten)]
    entry: LedgerEntry,
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

/// Whether two records are of one copy of one source.
fn equal(a: &LedgerEntry, b: &LedgerEntry) -> bool {
    a.id == b.id && a.src_hash == b.src_hash && a.dst_hash == b.dst_hash
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
        self.0.iter().any(|r| r.entry.id.as_deref() == Some(id))
    }

    /// How many archived copies of `vault` are there to come back.
    pub fn waiting(&self, vault: &Vault) -> usize {
        self.0
            .iter()
            .filter(|r| r.entry.of(&vault.root))
            .filter(|r| matches!(holds(vault, &r.path, r.entry.id.as_deref()), Ok(Some(_))))
            .count()
    }

    /// The record of `key`'s archived copy in `vault`.
    pub(super) fn of(&self, key: &str, vault: &Path) -> Option<&LedgerEntry> {
        self.0
            .iter()
            .find(|r| r.key == key && r.entry.of(vault))
            .map(|r| &r.entry)
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
            entry: LedgerEntry {
                from: Some(followed.to_path_buf()),
                vault: Some(vault.to_path_buf()),
                missing: false,
                ..entry.clone()
            },
        });
    }

    /// Brings the records and the ledger back into step with each other and with `vault`, after
    /// a pass cut short, a newer copy or the person's own hand. The only place a record is
    /// dropped. The ledger is saved first: a record moved to it and not yet dropped here is
    /// finished by the next pass, and the other order would lose it.
    pub(super) fn reconcile(
        &mut self,
        dirs: &Dirs,
        vault: &Vault,
        ledger: &mut Ledger,
    ) -> anyhow::Result<()> {
        let (mut in_ledger, mut in_archive) = (false, false);
        let mut purge = Vec::new();
        self.0.retain_mut(|r| {
            if !r.entry.of(&vault.root) {
                return true;
            }
            let id = r.entry.id.as_deref();
            let Ok(archived) = holds(vault, &r.path, id) else {
                return true;
            };
            let keep = match ledger.current(&r.key, &vault.root) {
                // Cut short after the move: the ledger has not let go of it yet.
                Some(l) if equal(l, &r.entry) && archived.is_some() => {
                    ledger.0.remove(&r.key);
                    in_ledger = true;
                    return true;
                }
                // Cut short before the move, or restored.
                Some(l) if equal(l, &r.entry) => false,
                // A newer copy has taken its place.
                Some(l) => {
                    if l.id != r.entry.id {
                        purge.extend(r.entry.id.clone());
                    }
                    false
                }
                None => {
                    let Ok(back) = holds(vault, &copy_of(vault, &r.key), id) else {
                        return true;
                    };
                    // While the archived file is still there, only the very bytes are it: a
                    // fresh copy written under the same id is not.
                    let back =
                        back.is_some_and(|b| archived.is_none() || hash(&b) == r.entry.dst_hash);
                    if back {
                        ledger.0.insert(r.key.clone(), r.entry.clone());
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
                        purge.extend(r.entry.id.clone());
                        ledger.0.insert(r.key.clone(), r.entry.clone());
                        in_ledger = true;
                        false
                    }
                }
            };
            in_archive |= !keep;
            keep
        });
        daemon::purge(dirs, purge.into_iter());
        if in_ledger {
            ledger.save(dirs)?;
        }
        if in_archive {
            self.save(dirs)?;
        }
        Ok(())
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
            let record = self.0.iter().find(|r| {
                r.key == source.key
                    && r.entry.of(&vault.root)
                    && r.entry.from.as_deref() == Some(followed)
            });
            let Some(r) = record.filter(|_| ledger.current(&source.key, &vault.root).is_none())
            else {
                left.push(source);
                continue;
            };
            // The person deleted the archived file, or it cannot be read: not copied again.
            let Ok(Some(archived)) = holds(vault, &r.path, r.entry.id.as_deref()) else {
                continue;
            };
            if hash(source.raw.as_bytes()) != r.entry.src_hash {
                returning.extend(r.entry.id.clone().map(|id| (source.key.clone(), id)));
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
            ledger.0.insert(source.key.clone(), r.entry.clone());
            // Only a copy as brain wrote it is settled: settling the person's edit would record
            // their words as brain's, and the next change would write over them.
            if hash(&archived) == r.entry.dst_hash {
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
