//! `brain memory follow`: the memory import run again and again, so a copy changes when its
//! source does and leaves search when its source is gone. Nothing is taken away on doubt: only a
//! copy this folder gave, untouched since, whose source two passes in a row found gone. A project
//! whose memory folder is now a link to another's was renamed, and its copies move with it, as do
//! those of a memory found unchanged under another project the moment its old place is empty. A
//! pass that would take most of what it follows takes nothing until the person says so.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::bail;
use brain_note::{Skip, Vault, rel_key};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{archive, is_link, rename_within, set_modified};
use brain_reflex::trace::{Call, Kind, Reply};
use serde_json::json;

use super::{
    Ledger, LedgerEntry, Source, Written, classify_sources, copy_rel, hash, memory, settle,
    write_one,
};

/// Gone memories archived without asking, when they are also most of those followed.
const UNASKED: usize = 10;

/// What one pass did, by vault path.
#[derive(Debug, Default)]
pub struct Followed {
    pub new: Vec<String>,
    pub updated: Vec<String>,
    pub archived: Vec<String>,
    /// Copies taken to a renamed project's name.
    pub moved: Vec<String>,
    /// Copies left as they are because the person's hand is on them.
    pub kept: Vec<String>,
    /// Sources that could not be taken, and why.
    pub skipped: Vec<(String, String)>,
    /// Why nothing gone was archived, and what to run if it should be.
    pub held: Option<String>,
}

impl Followed {
    pub fn summary(&self) -> String {
        format!(
            "{} copied, {} updated, {} archived, {} moved, {} kept as they are, {} skipped",
            self.new.len(),
            self.updated.len(),
            self.archived.len(),
            self.moved.len(),
            self.kept.len(),
            self.skipped.len()
        )
    }

    pub fn changed(&self) -> bool {
        [&self.new, &self.updated, &self.archived, &self.moved]
            .iter()
            .any(|changed| !changed.is_empty())
    }
}

/// A change a pass made, for its trace.
struct Change {
    what: &'static str,
    key: String,
    file: PathBuf,
    /// The note and its text, when the change put text in the vault. One that took text away
    /// holds neither: the trace would be purged with the note it records the going of.
    put: Option<(Option<String>, String)>,
}

/// One pass over `followed`; `told` when the person has just asked for this folder to be followed,
/// which lets it archive however much is gone. The caller holds the import lock. `index` brings the index up to
/// date with what was written, and the ledger is settled after it.
pub fn sync(
    dirs: &Dirs,
    vault: &Vault,
    followed: &Path,
    w: &Written<'_>,
    told: bool,
    index: impl FnOnce(&Vault) -> anyhow::Result<()>,
) -> anyhow::Result<Followed> {
    // A vault that is not there would be made again, empty but for the memories.
    if !vault.root.is_dir() {
        bail!(
            "the vault folder {} is not there; nothing changed",
            vault.root.display()
        );
    }
    if is_link(followed) || std::fs::read_dir(followed).is_err() {
        bail!(
            "the followed folder {} is not there to read; nothing changed",
            followed.display()
        );
    }
    let mut skipped = Vec::new();
    let all = memory::sources(followed, &vault.layout.memory, &mut skipped)?;
    let mut ledger = Ledger::load(dirs, vault)?;
    if all.is_empty() && ledger.holds_memory_of(followed, &vault.root) {
        bail!(
            "{} holds no memory file, while brain has copies from it; nothing changed",
            followed.display()
        );
    }
    let found: Vec<String> = all.iter().map(|s| s.key.clone()).collect();
    let mut out = Followed::default();
    let mut changes = Vec::new();
    renamed(
        vault,
        followed,
        &mut skipped,
        &found,
        &mut ledger,
        &mut out,
        &mut changes,
    );
    out.skipped
        .extend(skipped.into_iter().map(|(key, why)| (key, why.describe())));
    for s in &all {
        let free = ledger.current(&s.key, &vault.root).is_none()
            && std::fs::symlink_metadata(vault.root.join(&s.dst_rel)).is_err();
        if free && let Some(old) = left_empty(vault, followed, &ledger, &found, s) {
            carry(
                vault,
                followed,
                &mut ledger,
                &old,
                &s.key,
                &mut out,
                &mut changes,
            );
        }
    }
    let dated: Vec<(String, PathBuf, SystemTime)> = all
        .iter()
        .filter_map(|s| Some((s.key.clone(), s.dst_rel.clone(), s.modified?)))
        .collect();
    let mut plan = Default::default();
    let todo = classify_sources(all, vault, &ledger, &mut plan);
    out.kept.extend(plan.conflicts);
    let mut written = Vec::new();
    for (source, seen) in todo {
        let what = if seen.is_some() { "update" } else { "copy" };
        let (key, rel) = (source.key.clone(), rel_key(&source.dst_rel));
        let (file, text) = (source_of(followed, &key), source.raw.clone());
        match write_one(&source, seen, vault, &mut ledger, w) {
            Ok(()) => {
                let id = ledger.0.get(&key).and_then(|e| e.id.clone());
                changes.push(Change {
                    what,
                    key: key.clone(),
                    file,
                    put: Some((id, text)),
                });
                written.push(key);
                if what == "copy" {
                    out.new.push(rel);
                } else {
                    out.updated.push(rel);
                }
            }
            Err(e) => out.skipped.push((key, format!("{e:#}"))),
        }
    }
    // A source seen here is this folder's from now on, and is not missing.
    for key in &found {
        let present =
            copy_rel(&vault.layout, key).is_some_and(|rel| vault.root.join(rel).is_file());
        if let Some(entry) = ledger.0.get_mut(key)
            && entry.of(&vault.root)
        {
            entry.from = Some(followed.to_path_buf());
            entry.missing = false;
            // Only a copy that is here says which vault an older record is of.
            if present {
                entry.vault = Some(vault.root.clone());
            }
        }
    }
    date_back(vault, &ledger, &dated);
    gone(vault, followed, told, &mut ledger, &mut out, &mut changes);
    // Saved before the index runs, so a pass cut short never forgets what it wrote.
    ledger.save(dirs)?;
    trace(dirs, &changes);
    if out.changed() {
        index(vault)?;
        settle(&mut ledger, vault, &written)?;
        ledger.save(dirs)?;
    }
    Ok(out)
}

/// A copy brain made that says it changed after its source did, as one made before copies took
/// their source's date, is stamped with the source's. Never forward: a source only touched has
/// the words it had.
fn date_back(vault: &Vault, ledger: &Ledger, dated: &[(String, PathBuf, SystemTime)]) {
    for (key, rel, at) in dated {
        let copy = vault.root.join(rel);
        let later = std::fs::metadata(&copy)
            .and_then(|m| m.modified())
            .is_ok_and(|has| has > *at);
        let untouched = || {
            ledger.current(key, &vault.root).is_some_and(|entry| {
                std::fs::read(&copy).is_ok_and(|text| hash(&text) == entry.dst_hash)
            })
        };
        if later && untouched() {
            let _ = set_modified(&vault.root, rel, *at);
        }
    }
}

pub(super) fn source_of(followed: &Path, key: &str) -> PathBuf {
    let rest = key.strip_prefix("memory/").unwrap_or(key);
    match rest.split_once('/') {
        Some((project, file)) => followed.join(project).join("memory").join(file),
        None => followed.join(rest),
    }
}

/// Where the copy of the memory `key` is in the vault.
fn copy_of(vault: &Vault, key: &str) -> PathBuf {
    vault.layout.memory.join(key.trim_start_matches("memory/"))
}

/// A memory folder that is now a link to another project's folder here: Claude Code's folder was
/// renamed and left the link behind. Its copies move to the new name, so a memory stays one note,
/// with the person's edits and removals. Where the new name already has its own copy, the old one
/// is a second copy and goes as a gone source's would. Such a link is read where it leads, so it
/// is taken out of `skipped`.
fn renamed(
    vault: &Vault,
    followed: &Path,
    skipped: &mut Vec<(String, Skip)>,
    found: &[String],
    ledger: &mut Ledger,
    out: &mut Followed,
    changes: &mut Vec<Change>,
) {
    skipped.retain(|(link, why)| {
        if !matches!(why, Skip::Link) {
            return true;
        }
        let Some(target) = project_behind(followed, link) else {
            return true;
        };
        let under = format!("{link}/");
        let keys: Vec<String> = ledger
            .0
            .iter()
            .filter(|(key, e)| {
                key.starts_with(&under)
                    && e.of(&vault.root)
                    && e.from.as_deref().is_none_or(|from| from == followed)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            let new = format!("memory/{target}/{}", &key[under.len()..]);
            // Only a file read this pass is one to move to.
            if !found.contains(&new) {
                continue;
            }
            // Two records of one memory, as when a pass saw both names: the copy brain made later
            // gives way, unless the person has had their hand on it.
            let later = ledger.current(&new, &vault.root).is_some_and(|made| {
                made.id > ledger.0[&key].id
                    && std::fs::read(vault.root.join(copy_of(vault, &new)))
                        .is_ok_and(|copy| hash(&copy) == made.dst_hash)
            });
            if later {
                if !retire(vault, followed, &new, &ledger.0[&new], out, changes) {
                    continue;
                }
                ledger.forget(&new);
            }
            if ledger.current(&new, &vault.root).is_some() {
                if retire(vault, followed, &key, &ledger.0[&key], out, changes) {
                    ledger.forget(&key);
                }
                continue;
            }
            carry(vault, followed, ledger, &key, &new, out, changes);
        }
        false
    });
}

/// Takes `old`'s copy and record to `new`, the same memory under another project's name.
fn carry(
    vault: &Vault,
    followed: &Path,
    ledger: &mut Ledger,
    old: &str,
    new: &str,
    out: &mut Followed,
    changes: &mut Vec<Change>,
) {
    let (from, to) = (copy_of(vault, old), copy_of(vault, new));
    match rename_within(&vault.root, &from, &to) {
        Ok(()) => {
            out.moved.push(rel_key(&to));
            changes.push(Change {
                what: "move",
                key: new.into(),
                file: source_of(followed, new),
                put: None,
            });
        }
        // Nothing at the old place: the person removed it, or a pass cut short had moved it
        // already. The record moves either way.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        // The person's own file is where the copy would go.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if retire(vault, followed, old, &ledger.0[old], out, changes) {
                ledger.forget(old);
            }
            return;
        }
        Err(e) => {
            out.skipped.push((old.into(), format!("not moved: {e}")));
            return;
        }
    }
    if let Some(entry) = ledger.0.remove(old) {
        ledger.0.insert(new.into(), entry);
    }
}

/// The record `source` is the same memory as: same file name and words, under another project
/// where it is no longer found. A project moved with no link, or not yet linked, looks like this.
fn left_empty(
    vault: &Vault,
    followed: &Path,
    ledger: &Ledger,
    found: &[String],
    source: &Source,
) -> Option<String> {
    let words = hash(source.raw.as_bytes());
    let name = source.key.rsplit('/').next()?;
    ledger
        .0
        .iter()
        .find(|(key, e)| {
            key.starts_with("memory/")
                && key.rsplit('/').next() == Some(name)
                && e.src_hash == words
                && e.of(&vault.root)
                && e.from.as_deref().is_none_or(|from| from == followed)
                && !found.contains(key)
                && not_found(followed, key)
        })
        .map(|(key, _)| key.clone())
}

/// Whether nothing is where `key`'s source was: any other answer means unread, not gone.
fn not_found(followed: &Path, key: &str) -> bool {
    matches!(
        std::fs::symlink_metadata(source_of(followed, key)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    )
}

/// The project whose memory folder `link`, a `memory/<project>` the pass did not follow, leads to
/// among the followed projects.
fn project_behind(followed: &Path, link: &str) -> Option<String> {
    let project = link.strip_prefix("memory/")?;
    let target = followed.join(project).join("memory").canonicalize().ok()?;
    let beside = target.strip_prefix(followed).ok()?;
    let name = beside
        .parent()?
        .to_str()
        .filter(|n| !n.contains(['/', '\\']))?;
    (beside.ends_with("memory") && !name.is_empty()).then(|| name.to_string())
}

/// Takes the copy of a source that is no more: archived as the ledger wrote it, kept and said when
/// the person's hand is on it. False when it could not be dealt with, so the next pass tries again.
fn retire(
    vault: &Vault,
    followed: &Path,
    key: &str,
    entry: &LedgerEntry,
    out: &mut Followed,
    changes: &mut Vec<Change>,
) -> bool {
    let rel = copy_of(vault, key);
    match std::fs::read(vault.root.join(&rel)) {
        Ok(copy) if hash(&copy) == entry.dst_hash => {
            if let Err(e) = archive(&vault.root, &rel, &vault.layout.archive) {
                out.skipped.push((key.into(), format!("not archived: {e}")));
                return false;
            }
            out.archived.push(rel_key(&rel));
            changes.push(Change {
                what: "archive",
                key: key.into(),
                file: source_of(followed, key),
                put: None,
            });
        }
        // The person's own words now: kept, said once and no longer followed.
        Ok(_) => out.kept.push(rel_key(&rel)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    true
}

/// Deals with the copies whose source is gone: not found, on this pass and the one before. Most of
/// the followed memories going at once looks like a folder reset or replaced, so then nothing is
/// taken until the person says so.
fn gone(
    vault: &Vault,
    followed: &Path,
    told: bool,
    ledger: &mut Ledger,
    out: &mut Followed,
    changes: &mut Vec<Change>,
) {
    let (mut all, mut ripe) = (0, Vec::new());
    for (key, entry) in &mut ledger.0 {
        if !key.starts_with("memory/")
            || entry.from.as_deref() != Some(followed)
            || !entry.of(&vault.root)
        {
            continue;
        }
        all += 1;
        let gone = not_found(followed, key);
        if std::mem::replace(&mut entry.missing, gone) && gone {
            ripe.push(key.clone());
        }
    }
    if !told && ripe.len() > UNASKED && ripe.len() * 2 > all {
        out.held = Some(format!(
            "{} of {all} followed memories are gone from {followed}; none archived. If that is right, run `brain memory follow \"{followed}\"`",
            ripe.len(),
            followed = followed.display(),
        ));
        return;
    }
    for key in ripe {
        if retire(vault, followed, &key, &ledger.0[&key], out, changes) {
            ledger.forget(&key);
        }
    }
}

/// Records a pass that changed something, so each change can be read afterwards. A trace that
/// cannot be written never stops the pass.
fn trace(dirs: &Dirs, changes: &[Change]) {
    if changes.is_empty() {
        return;
    }
    let mut run = brain_reflex::record(&dirs.data, "memory follow");
    for change in changes {
        let _ = run.span(Kind::Step, change.what, |run| {
            let request = json!({ "key": change.key });
            let mut call = Call::new(Kind::Read, "source", &request);
            let mut sources = vec![format!("file:{}", change.file.display())];
            let (id, text) = change
                .put
                .as_ref()
                .map_or((None, None), |(id, text)| (id.as_ref(), Some(text)));
            sources.extend(id.map(|id| format!("note:{id}")));
            let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
            call.sources = &sources;
            let _ = run.call(call, || Ok(Reply::new(json!(text))));
            Ok(())
        });
    }
    if let Err(e) = run.finish() {
        eprintln!("brain: this memory pass was not fully traced: {e}");
    }
}
