//! `brain memory follow`: the memory import run again and again, so a copy changes when its
//! source does and leaves search when its source is gone. Nothing is taken away on doubt: only a
//! copy this folder gave, untouched since, whose source two passes in a row found gone. Nothing is
//! taken away for good: an archived copy comes back, with its id, when its source does. A project
//! whose memory folder is now a link to another's was renamed, and its copies move with it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::bail;
use brain_note::{Skip, Vault, rel_key};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{is_link, rename_within, set_modified};
use brain_reflex::trace::{Call, Kind, Reply};
use serde_json::json;

use super::{Archive, Ledger, Written, classify_sources, hash, memory, settle, write_one};

/// What one pass did, by vault path.
#[derive(Debug, Default)]
pub struct Followed {
    pub new: Vec<String>,
    pub updated: Vec<String>,
    pub archived: Vec<String>,
    /// Copies brought back because their source returned.
    pub restored: Vec<String>,
    /// Copies taken to a renamed project's name.
    pub moved: Vec<String>,
    /// Copies left as they are because the person's hand is on them.
    pub kept: Vec<String>,
    /// Sources that could not be taken, and why.
    pub skipped: Vec<(String, String)>,
    /// Each change made, for the pass's trace.
    pub(super) changes: Vec<Change>,
}

impl Followed {
    pub fn summary(&self) -> String {
        format!(
            "{} copied, {} updated, {} archived, {} moved, {} restored, {} kept as they are, {} skipped",
            self.new.len(),
            self.updated.len(),
            self.archived.len(),
            self.moved.len(),
            self.restored.len(),
            self.kept.len(),
            self.skipped.len()
        )
    }

    pub fn changed(&self) -> bool {
        [
            &self.new,
            &self.updated,
            &self.archived,
            &self.moved,
            &self.restored,
        ]
        .iter()
        .any(|changed| !changed.is_empty())
    }
}

/// A change a pass made, for its trace.
#[derive(Debug)]
pub(super) struct Change {
    pub(super) what: &'static str,
    pub(super) key: String,
    pub(super) file: PathBuf,
    /// The note and its text, when the change put text in the vault. One that took text away
    /// holds neither: the trace would be purged with the note it records the going of.
    pub(super) put: Option<(Option<String>, String)>,
}

/// One pass over `followed`. The caller holds the import lock. `index` brings the index up to
/// date with what was written, and the ledger is settled after it.
pub fn sync(
    dirs: &Dirs,
    vault: &Vault,
    followed: &Path,
    w: &Written<'_>,
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
    let mut ledger = Ledger::load(dirs)?;
    if all.is_empty() && ledger.holds_memory_of(followed, vault) {
        bail!(
            "{} holds no memory file, while brain has copies from it; nothing changed",
            followed.display()
        );
    }
    let found: Vec<String> = all.iter().map(|s| s.key.clone()).collect();
    let mut archive = Archive::load(dirs)?;
    archive.reconcile(dirs, vault, &mut ledger)?;
    let mut out = Followed::default();
    let second = renamed(
        vault,
        followed,
        &mut skipped,
        &found,
        &archive,
        &mut ledger,
        &mut out,
    );
    // Projects this pass did not read, as one behind a link: what is under them is not gone.
    let unread: Vec<String> = skipped
        .iter()
        .filter(|(key, _)| key.matches('/').count() == 1)
        .map(|(key, _)| format!("{key}/"))
        .collect();
    out.skipped
        .extend(skipped.into_iter().map(|(key, why)| (key, why.describe())));
    let dated: Vec<(String, PathBuf, SystemTime)> = all
        .iter()
        .filter_map(|s| Some((s.key.clone(), s.dst_rel.clone(), s.modified?)))
        .collect();
    let mut written = Vec::new();
    let (all, returning) =
        archive.restore(vault, followed, all, &mut ledger, &mut written, &mut out);
    if !out.restored.is_empty() {
        ledger.save(dirs)?;
    }
    let mut plan = Default::default();
    let todo = classify_sources(all, vault, &ledger, &mut plan);
    out.kept.extend(plan.conflicts);
    for (source, seen) in todo {
        let what = if seen.is_some() { "update" } else { "copy" };
        let (key, rel) = (source.key.clone(), rel_key(&source.dst_rel));
        let (file, text) = (source_of(followed, &key), source.raw.clone());
        let was = returning.get(&key).cloned();
        match write_one(&source, seen, was, vault, &mut ledger, w) {
            Ok(()) => {
                let id = ledger.0.get(&key).and_then(|e| e.id.clone());
                out.changes.push(Change {
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
        if let Some(entry) = ledger.0.get_mut(key)
            && entry.of(&vault.root)
        {
            entry.from = Some(followed.to_path_buf());
            entry.vault = Some(vault.root.clone());
            entry.missing = false;
        }
    }
    date_back(vault, &ledger, &dated);
    archive_due(
        dirs,
        vault,
        followed,
        Look { second, unread },
        &mut ledger,
        &mut archive,
        &mut out,
    )?;
    // Saved before the index runs, so a pass cut short there never forgets what it wrote.
    ledger.save(dirs)?;
    trace(dirs, &out.changes);
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

/// Where the copy of `key` is in the vault.
pub(super) fn copy_of(vault: &Vault, key: &str) -> PathBuf {
    vault.layout.memory.join(key.trim_start_matches("memory/"))
}

/// A memory folder that is now a link to another project's folder here: Claude Code's folder was
/// renamed and left the link behind. Its copies move to the new name, so a memory stays one note,
/// with the person's edits and removals. Where the new name already has its own copy, archived or
/// not, the old one is a second copy and goes as a gone source's would. Such a link is read where
/// it leads, so it is taken out of `skipped`.
fn renamed(
    vault: &Vault,
    followed: &Path,
    skipped: &mut Vec<(String, Skip)>,
    found: &[String],
    archive: &Archive,
    ledger: &mut Ledger,
    out: &mut Followed,
) -> Vec<String> {
    let mut second = Vec::new();
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
            let (from, to) = (copy_of(vault, &key), copy_of(vault, &new));
            let there = |rel: &Path| std::fs::symlink_metadata(vault.root.join(rel)).is_ok();
            // A copy takes the new name only where nothing would then be written over it: the
            // name has no record of its own, or one of the same words. An archived copy's record
            // is the name's record too: the name has had a memory of its own.
            let theirs = ledger
                .current(&new, &vault.root)
                .or_else(|| archive.of(&new, &vault.root));
            let same = theirs.is_none_or(|e| e.src_hash == ledger.0[&key].src_hash);
            match (there(&from), there(&to) || !same) {
                // The new name has its own: the old name's copy is archived, as a gone one is.
                (true, true) => {
                    second.push(key);
                    continue;
                }
                (true, false) => match rename_within(&vault.root, &from, &to) {
                    Ok(()) => {
                        out.moved.push(rel_key(&to));
                        out.changes.push(Change {
                            what: "move",
                            key: new.clone(),
                            file: source_of(followed, &new),
                            put: None,
                        });
                    }
                    Err(e) => {
                        out.skipped.push((key, format!("not moved: {e}")));
                        continue;
                    }
                },
                // Nothing at the old place, and the new name has its own record.
                (false, _) if theirs.is_some() => {
                    ledger.0.remove(&key);
                    continue;
                }
                // The person removed it, or a pass cut short had moved it: the record moves.
                (false, _) => {}
            }
            if let Some(entry) = ledger.0.remove(&key) {
                ledger.0.insert(new, entry);
            }
        }
        false
    });
    second
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

/// What a pass found that bears on archiving.
struct Look {
    /// Copies of memories that already have one under another name.
    second: Vec<String>,
    /// `memory/<project>/` of each project not read.
    unread: Vec<String>,
}

/// The one way a copy is archived. Due are those whose source was not found on this pass and the
/// one before, and the `second` copies of memories already under another name. Each is recorded,
/// and the record saved, before any file moves, so a pass cut short never loses where one went.
fn archive_due(
    dirs: &Dirs,
    vault: &Vault,
    followed: &Path,
    look: Look,
    ledger: &mut Ledger,
    archive: &mut Archive,
    out: &mut Followed,
) -> anyhow::Result<()> {
    let mut due = Vec::new();
    for (key, entry) in &mut ledger.0 {
        if look.second.contains(key) {
            due.push(key.clone());
            continue;
        }
        if !key.starts_with("memory/")
            || entry.from.as_deref() != Some(followed)
            || !entry.of(&vault.root)
            || look.unread.iter().any(|project| key.starts_with(project))
        {
            continue;
        }
        // Any other answer than "not found" means unread, not gone.
        let gone = matches!(
            std::fs::symlink_metadata(source_of(followed, key)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
        );
        if std::mem::replace(&mut entry.missing, gone) && gone {
            due.push(key.clone());
        }
    }
    let mut moves = Vec::new();
    for key in due {
        let rel = copy_of(vault, &key);
        let entry = &ledger.0[&key];
        match std::fs::read(vault.root.join(&rel)) {
            Ok(copy) if hash(&copy) == entry.dst_hash => {
                let to = archive.name_for(vault, &rel);
                archive.add(&key, to.clone(), entry, followed, &vault.root);
                moves.push((key, rel, to));
            }
            // The person's own words now: kept, said once and no longer followed.
            Ok(_) => {
                out.kept.push(rel_key(&rel));
                ledger.0.remove(&key);
            }
            // Removed by the person: the record stays, so it is not copied again.
            Err(_) => {}
        }
    }
    if moves.is_empty() {
        return Ok(());
    }
    archive.save(dirs)?;
    for (key, rel, to) in moves {
        if let Err(e) = rename_within(&vault.root, &rel, &to) {
            out.skipped.push((key, format!("not archived: {e}")));
            continue;
        }
        ledger.0.remove(&key);
        out.archived.push(rel_key(&rel));
        out.changes.push(Change {
            what: "archive",
            file: source_of(followed, &key),
            key,
            put: None,
        });
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use brain_platform::fs::{Privacy, write_atomic};

    use super::*;
    use crate::import::LedgerEntry;

    /// The record file cannot be written, so what happened to the copy is what happens before it
    /// is saved.
    #[test]
    fn nothing_is_moved_before_its_record_is_saved() {
        let home = tempfile::tempdir().unwrap();
        let dirs = Dirs::from_home(home.path());
        let put = |rel: &str| {
            let rel = Path::new(rel);
            write_atomic(home.path(), rel, b"words\n", Privacy::Private).unwrap();
        };
        // A folder where the record file goes.
        put("data/follow-archive.json/x");
        put("vault/working-memory/-proj/one.md");
        let vault = Vault::open(&home.path().join("vault")).unwrap();
        let copy = vault.root.join("working-memory/-proj/one.md");
        let key = "memory/-proj/one.md".to_string();
        let mut ledger = Ledger::default();
        ledger.0.insert(
            key.clone(),
            LedgerEntry {
                id: Some("0195f2e0-0000-7000-8000-000000000001".into()),
                src_hash: hash(b"source"),
                dst_hash: hash(b"words\n"),
                from: None,
                vault: None,
                missing: false,
            },
        );
        let done = archive_due(
            &dirs,
            &vault,
            home.path(),
            Look {
                second: vec![key.clone()],
                unread: Vec::new(),
            },
            &mut ledger,
            &mut Archive::default(),
            &mut Followed::default(),
        );
        assert!(done.is_err());
        assert!(copy.exists(), "moved before its record was saved");
        assert!(ledger.0.contains_key(&key));
    }
}
