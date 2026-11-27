//! `brain memory follow`: the memory import run again and again, so a copy changes when its
//! source does and leaves search when its source is gone. Nothing is taken away on doubt: only a
//! copy this folder gave, untouched since, whose source two passes in a row found gone.

use std::path::{Path, PathBuf};

use anyhow::bail;
use brain_note::{Vault, rel_key};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{archive, is_link};
use brain_reflex::trace::{Call, Kind, Reply};
use serde_json::json;

use super::{Ledger, Written, classify_sources, hash, memory, settle, write_one};

/// What one pass did, by vault path.
#[derive(Debug, Default)]
pub struct Followed {
    pub new: Vec<String>,
    pub updated: Vec<String>,
    pub archived: Vec<String>,
    /// Copies left as they are because the person's hand is on them.
    pub kept: Vec<String>,
    /// Sources that could not be taken, and why.
    pub skipped: Vec<(String, String)>,
}

impl Followed {
    pub fn summary(&self) -> String {
        format!(
            "{} copied, {} updated, {} archived, {} kept as they are, {} skipped",
            self.new.len(),
            self.updated.len(),
            self.archived.len(),
            self.kept.len(),
            self.skipped.len()
        )
    }

    pub fn changed(&self) -> bool {
        !(self.new.is_empty() && self.updated.is_empty() && self.archived.is_empty())
    }
}

/// A change a pass made, for its trace.
struct Change {
    what: &'static str,
    key: String,
    file: PathBuf,
    id: Option<String>,
    text: String,
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
    if all.is_empty() && ledger.holds_memory_of(followed, &vault.root) {
        bail!(
            "{} holds no memory file, while brain has copies from it; nothing changed",
            followed.display()
        );
    }
    let mut out = Followed {
        skipped: skipped
            .into_iter()
            .map(|(key, why)| (key, why.describe()))
            .collect(),
        ..Followed::default()
    };
    let found: Vec<String> = all.iter().map(|s| s.key.clone()).collect();
    let mut plan = Default::default();
    let todo = classify_sources(all, vault, &ledger, &mut plan);
    out.kept = plan.conflicts;
    let mut changes = Vec::new();
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
                    id,
                    text,
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
    gone(vault, followed, &mut ledger, &mut out, &mut changes);
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

fn source_of(followed: &Path, key: &str) -> PathBuf {
    let rest = key.strip_prefix("memory/").unwrap_or(key);
    match rest.split_once('/') {
        Some((project, file)) => followed.join(project).join("memory").join(file),
        None => followed.join(rest),
    }
}

/// Deals with the copies whose source is gone: not found, on this pass and the one before.
fn gone(
    vault: &Vault,
    followed: &Path,
    ledger: &mut Ledger,
    out: &mut Followed,
    changes: &mut Vec<Change>,
) {
    let mut forget = Vec::new();
    for (key, entry) in &mut ledger.0 {
        if !key.starts_with("memory/")
            || entry.from.as_deref() != Some(followed)
            || !entry.of(&vault.root)
        {
            continue;
        }
        let file = source_of(followed, key);
        // Any other answer than "not found" means unread, not gone.
        let not_found = matches!(
            std::fs::symlink_metadata(&file),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
        );
        if !not_found || !std::mem::replace(&mut entry.missing, true) {
            entry.missing = not_found;
            continue;
        }
        let rel = vault.layout.memory.join(key.trim_start_matches("memory/"));
        match std::fs::read(vault.root.join(&rel)) {
            Ok(copy) if hash(&copy) == entry.dst_hash => {
                if let Err(e) = archive(&vault.root, &rel, &vault.layout.archive) {
                    // Kept in the ledger, so the next pass tries again.
                    out.skipped
                        .push((key.clone(), format!("not archived: {e}")));
                    continue;
                }
                out.archived.push(rel_key(&rel));
                changes.push(Change {
                    what: "archive",
                    key: key.clone(),
                    file,
                    id: entry.id.clone(),
                    text: String::from_utf8_lossy(&copy).into_owned(),
                });
            }
            // The person's own words now: kept, said once and no longer followed.
            Ok(_) => out.kept.push(rel_key(&rel)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => continue,
        }
        forget.push(key.clone());
    }
    for key in forget {
        ledger.0.remove(&key);
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
            sources.extend(change.id.as_ref().map(|id| format!("note:{id}")));
            let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
            call.sources = &sources;
            let _ = run.call(call, || Ok(Reply::new(json!(change.text))));
            Ok(())
        });
    }
    if let Err(e) = run.finish() {
        eprintln!("brain: this memory pass was not fully traced: {e}");
    }
}
