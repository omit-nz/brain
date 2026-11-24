//! A vault: the folder of notes and the walk over it. The walk never follows a link and says why
//! it skipped what it skipped, so a note that is not indexed is never a silent mystery.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, bail};
use brain_platform::fs::is_link;

use crate::disclosure::Place;

/// A note larger than this is skipped: it is almost certainly not a note someone wrote.
pub const MAX_NOTE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    Link,
    TooLarge(u64),
    NotUtf8,
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Note { rel: PathBuf, place: Place },
    Skipped { rel: PathBuf, why: Skip },
}

/// The folders that mean something. learning-loop's numbered folders need no field: the walk
/// treats them like any other kept folder.
pub struct Layout {
    pub inbox: PathBuf,
    pub private: PathBuf,
    pub archive: PathBuf,
    pub memory: PathBuf,
}

impl Default for Layout {
    fn default() -> Layout {
        Layout {
            inbox: "0-inbox".into(),
            private: "private".into(),
            archive: "_archive".into(),
            memory: "working-memory".into(),
        }
    }
}

pub struct Vault {
    pub root: PathBuf,
    pub layout: Layout,
}

impl Vault {
    /// Opens the folder at `root`, refusing one that is itself a link: a vault is read where it
    /// lives, never through a pointer someone could swap.
    pub fn open(root: &Path) -> anyhow::Result<Vault> {
        if is_link(root) {
            bail!("{} is a link; open the folder it points to", root.display());
        }
        let root = root
            .canonicalize()
            .with_context(|| format!("no vault at {}", root.display()))?;
        if !root.is_dir() {
            bail!("{} is not a folder", root.display());
        }
        Ok(Vault {
            root,
            layout: Layout::default(),
        })
    }

    /// Every `.md` file, in a stable order, skipping dot folders and files, underscore folders
    /// and the archive.
    pub fn walk(&self) -> Walk<'_> {
        Walk {
            vault: self,
            inner: walkdir::WalkDir::new(&self.root)
                .follow_links(false)
                .sort_by_file_name()
                .into_iter(),
        }
    }

    fn place(&self, rel: &Path) -> Place {
        if rel.starts_with(&self.layout.inbox) {
            Place::Inbox
        } else if rel.starts_with(&self.layout.private) {
            Place::Private
        } else {
            Place::Kept
        }
    }

    /// The note's text, refusing a path that leaves the vault or passes through a link, a file
    /// over `MAX_NOTE_BYTES`, and bytes that are not UTF-8.
    pub fn read(&self, rel: &Path) -> Result<String, Skip> {
        let mut at = self.root.clone();
        for c in rel.components() {
            let Component::Normal(part) = c else {
                return Err(Skip::Unreadable(format!(
                    "{} leaves the vault",
                    rel.display()
                )));
            };
            at.push(part);
            if is_link(&at) {
                return Err(Skip::Link);
            }
        }
        let unreadable = |e: std::io::Error| Skip::Unreadable(e.to_string());
        let len = fs::metadata(&at).map_err(unreadable)?.len();
        if len > MAX_NOTE_BYTES {
            return Err(Skip::TooLarge(len));
        }
        let bytes = fs::read(&at).map_err(unreadable)?;
        if bytes.len() as u64 > MAX_NOTE_BYTES {
            return Err(Skip::TooLarge(bytes.len() as u64));
        }
        String::from_utf8(bytes).map_err(|_| Skip::NotUtf8)
    }
}

pub struct Walk<'v> {
    vault: &'v Vault,
    inner: walkdir::IntoIter,
}

impl Walk<'_> {
    fn rel(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.vault.root)
            .unwrap_or(path)
            .to_path_buf()
    }
}

impl Iterator for Walk<'_> {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        loop {
            let entry = match self.inner.next()? {
                Ok(entry) => entry,
                Err(e) => {
                    let rel = e.path().map(|p| self.rel(p)).unwrap_or_default();
                    let why = Skip::Unreadable(e.to_string());
                    return Some(Entry::Skipped { rel, why });
                }
            };
            if entry.depth() == 0 {
                continue;
            }
            let rel = self.rel(entry.path());
            let name = entry.file_name().to_string_lossy();
            let ft = entry.file_type();
            let hidden = name.starts_with('.')
                || (ft.is_dir() && name.starts_with('_'))
                || rel == self.vault.layout.archive;
            if hidden {
                if ft.is_dir() {
                    self.inner.skip_current_dir();
                }
                continue;
            }
            if ft.is_symlink() {
                return Some(Entry::Skipped {
                    rel,
                    why: Skip::Link,
                });
            }
            if !ft.is_file() || entry.path().extension().is_none_or(|e| e != "md") {
                continue;
            }
            let why = match entry.metadata() {
                Ok(m) if m.len() > MAX_NOTE_BYTES => Skip::TooLarge(m.len()),
                Ok(_) => {
                    let place = self.vault.place(&rel);
                    return Some(Entry::Note { rel, place });
                }
                Err(e) => Skip::Unreadable(e.to_string()),
            };
            return Some(Entry::Skipped { rel, why });
        }
    }
}
