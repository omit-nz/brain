//! Importing a learning-loop brain: its vault and its auto-memory, copied into the brain vault.
//! The source is only read. Every note comes across private unless it declares otherwise or sits
//! in a folder the person chose to share and learning-loop's own rules do not withhold it.
//! A ledger remembers what was copied, so a second import updates only what changed and never
//! overwrites a copy the person edited.

mod archived;
mod blocklist;
mod follow;
mod memory;
mod report;
mod vault;

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, bail};
use brain_note::{Layout, Place, Skip, Vault, rel_key};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, replace_unchanged, set_modified, write_atomic};
use jiff::ToSpan;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use archived::{Archive, archived_line};
pub use blocklist::Blocklist;
pub use follow::{Followed, sync};
pub use report::report;

/// Days over which first reviews are spread.
const REVIEW_SPREAD: i64 = 90;
pub const LEDGER: &str = "import-ledger.json";
/// Held by whoever reads, changes and saves the ledger.
pub const LOCK: &str = "import.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: Option<String>,
    pub src_hash: String,
    pub dst_hash: String,
    /// The folder the source was found in, a memory import's projects folder or the vault imported
    /// from; unknown for a copy made before this was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<PathBuf>,
    /// The vault the copy is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault: Option<PathBuf>,
    /// The last pass did not find the source.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}

impl LedgerEntry {
    /// Whether this records a copy in `vault`; an entry that names no vault counts as this one.
    fn of(&self, vault: &Path) -> bool {
        self.vault.as_deref().is_none_or(|v| v == vault)
    }
}

/// What each import copied, keyed by the source's key (`vault/<path>` or
/// `memory/<project>/<file>`).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ledger(BTreeMap<String, LedgerEntry>);

impl Ledger {
    pub fn load(dirs: &Dirs) -> anyhow::Result<Ledger> {
        let path = dirs.data.join(LEDGER);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("reading {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ledger::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// What was recorded of `key`'s copy in `vault`; a copy in another vault is no record.
    fn current(&self, key: &str, vault: &Path) -> Option<&LedgerEntry> {
        self.0.get(key).filter(|e| e.of(vault))
    }

    /// The source of the copy at `path` in `vault`, when that file no longer reads as it did when
    /// copied. A source that is gone or cannot be read says nothing, and so does a copy whose
    /// source folder was never recorded.
    pub fn moved_on(&self, vault: &Vault, path: &str) -> Option<PathBuf> {
        let (entry, source) = match self.current(&format!("vault/{path}"), &vault.root) {
            Some(entry) => (entry, entry.from.as_ref()?.join(path)),
            None => {
                let rest = Path::new(path).strip_prefix(&vault.layout.memory).ok()?;
                let key = format!("memory/{}", rel_key(rest));
                let entry = self.current(&key, &vault.root)?;
                (entry, follow::source_of(entry.from.as_ref()?, &key))
            }
        };
        let now = std::fs::read(&source).ok()?;
        (hash(&now) != entry.src_hash).then_some(source)
    }

    /// How many copies still in `vault` are behind their source.
    pub fn behind(&self, vault: &Vault) -> usize {
        self.0
            .keys()
            .filter_map(|key| match key.split_once('/')? {
                ("vault", path) => Some(path.to_string()),
                ("memory", rest) => Some(rel_key(&vault.layout.memory.join(rest))),
                _ => None,
            })
            .filter(|path| vault.root.join(path).is_file() && self.moved_on(vault, path).is_some())
            .count()
    }

    /// Whether a memory copy that is still there came from `folder`, or may have.
    fn holds_memory_of(&self, folder: &Path, vault: &Vault) -> bool {
        self.0.iter().any(|(key, e)| {
            key.starts_with("memory/")
                && e.of(&vault.root)
                && e.from.as_deref().is_none_or(|from| from == folder)
                && vault.root.join(follow::copy_of(vault, key)).is_file()
        })
    }

    pub fn save(&self, dirs: &Dirs) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        write_atomic(
            &dirs.data,
            Path::new(LEDGER),
            text.as_bytes(),
            Privacy::Private,
        )?;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct ImportPlan {
    pub new: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: usize,
    pub conflicts: Vec<String>,
    pub skipped: Vec<(String, Skip)>,
    /// Where every source's copy goes, written this run or not.
    pub scope: HashSet<String>,
}

/// A source to write, with the hash of its copy as classified (`None`: no copy yet).
type Todo = (Source, Option<String>);

/// One file the import reads and where its copy goes.
pub struct Source {
    key: String,
    dst_rel: PathBuf,
    raw: String,
    memory: bool,
    /// The memory folder it was found in.
    from: Option<PathBuf>,
    /// When the source was last changed; its copy says the same.
    modified: Option<SystemTime>,
}

/// What every copy is written with.
pub struct Written<'a> {
    pub me: &'a str,
    pub today: Date,
    pub share: &'a [PathBuf],
    pub blocklist: Option<&'a Blocklist>,
    pub layout: &'a Layout,
}

impl<'a> Written<'a> {
    /// What a memory copy is written with: it is private, so no share rule applies.
    pub fn for_memory(me: &'a str, today: Date, vault: &'a Vault) -> Written<'a> {
        Written {
            me,
            today,
            share: &[],
            blocklist: None,
            layout: &vault.layout,
        }
    }

    /// Visibility for a note that declares none: the inbox and the private folder stay private
    /// whatever is shared, so a kept inbox note is not shared on the way out; then a learning-loop
    /// rule that withholds it wins, then a shared folder; everything else is private.
    fn visibility(&self, rel: &Path) -> &'static str {
        if self.layout.place(rel) != Place::Kept
            || self.blocklist.is_some_and(|b| b.withholds(&rel_key(rel)))
        {
            "private"
        } else if self.share.iter().any(|f| rel.starts_with(f)) {
            "shared"
        } else {
            "private"
        }
    }

    /// A first review between 1 and 90 days out, chosen by a hash of `seed`, so a large import
    /// does not land every review on one day.
    fn review_date(&self, seed: &str) -> Date {
        let h = Sha256::digest(seed.as_bytes());
        let n = u64::from_le_bytes(h[..8].try_into().expect("8 bytes")) % REVIEW_SPREAD as u64;
        self.today
            .checked_add((n as i64 + 1).days())
            .unwrap_or(self.today)
    }
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The `--share` folders as vault paths, with `./` and a trailing `/` dropped. A folder is refused
/// when it is empty or the vault itself, absolute, climbs out with `..`, or is not a folder of the
/// source spelled exactly as it is there, since the match is exact and a near miss would
/// silently share nothing.
pub fn share_folders(src: &Vault, share: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    share
        .iter()
        .map(|f| {
            let mut rel = PathBuf::new();
            for c in f.components() {
                match c {
                    Component::CurDir => {}
                    Component::Normal(name) => rel.push(name),
                    _ => bail!("--share {}: name a folder inside the vault", f.display()),
                }
            }
            if rel.as_os_str().is_empty() {
                bail!(
                    "--share {:?}: name a folder inside the vault",
                    f.display().to_string()
                );
            }
            let mut dir = src.root.clone();
            for name in rel.iter() {
                let found = std::fs::read_dir(&dir)
                    .with_context(|| format!("reading {}", dir.display()))?
                    .filter_map(Result::ok)
                    .any(|e| e.file_name() == name && e.file_type().is_ok_and(|t| t.is_dir()));
                if !found {
                    bail!(
                        "--share {}: {} has no folder {}",
                        f.display(),
                        src.root.display(),
                        rel_key(&rel)
                    );
                }
                dir.push(name);
            }
            Ok(rel)
        })
        .collect()
}

/// Refuses a source that is the brain vault or inside it, or holds it.
pub fn check_apart(src: &Vault, dst: &Vault) -> anyhow::Result<()> {
    if src.root.starts_with(&dst.root) || dst.root.starts_with(&src.root) {
        bail!(
            "{} and the brain vault {} overlap; import from a separate folder",
            src.root.display(),
            dst.root.display()
        );
    }
    Ok(())
}

fn sources(
    src: &Vault,
    memory: Option<&Path>,
    dst: &Vault,
    skipped: &mut Vec<(String, Skip)>,
) -> anyhow::Result<Vec<Source>> {
    let mut out = vault::sources(src, skipped);
    if let Some(root) = memory {
        out.extend(memory::sources(root, &dst.layout.memory, skipped)?);
    }
    Ok(out)
}

/// The plan and the sources to write (new and updated). A copy is compared with the ledger's
/// record of what was written: new when there is none (and nothing at its path), updated when
/// only the source changed, a conflict when both changed or an unrecorded file sits at its path.
fn classify(
    src: &Vault,
    memory: Option<&Path>,
    dst: &Vault,
    ledger: &Ledger,
) -> anyhow::Result<(ImportPlan, Vec<Todo>)> {
    let mut plan = ImportPlan::default();
    let all = sources(src, memory, dst, &mut plan.skipped)?;
    let todo = classify_sources(all, dst, ledger, &mut plan);
    Ok((plan, todo))
}

fn classify_sources(
    all: Vec<Source>,
    dst: &Vault,
    ledger: &Ledger,
    plan: &mut ImportPlan,
) -> Vec<Todo> {
    let mut todo = Vec::new();
    let mut claimed = HashSet::new();
    for s in all {
        plan.scope.insert(rel_key(&s.dst_rel));
        let src_hash = hash(s.raw.as_bytes());
        let dst_path = dst.root.join(&s.dst_rel);
        let dst_hash = std::fs::read(&dst_path).ok().map(|b| hash(&b));
        // Vault notes come first, so a vault note under working-memory/ keeps its path and the
        // memory note meant for it is the conflict.
        let first = claimed.insert(s.dst_rel.clone());
        match ledger.current(&s.key, &dst.root) {
            _ if !first => plan.conflicts.push(rel_key(&s.dst_rel)),
            None if dst_hash.is_some() => plan.conflicts.push(rel_key(&s.dst_rel)),
            None => {
                plan.new.push(rel_key(&s.dst_rel));
                todo.push((s, dst_hash));
            }
            Some(e) if e.src_hash == src_hash => plan.unchanged += 1,
            Some(e) if dst_hash.as_deref() == Some(e.dst_hash.as_str()) => {
                plan.updated.push(rel_key(&s.dst_rel));
                todo.push((s, dst_hash));
            }
            Some(_) => plan.conflicts.push(rel_key(&s.dst_rel)),
        }
    }
    todo
}

pub fn plan(
    src: &Vault,
    memory: Option<&Path>,
    dst: &Vault,
    ledger: &Ledger,
) -> anyhow::Result<ImportPlan> {
    Ok(classify(src, memory, dst, ledger)?.0)
}

/// Classifies, then writes every new and updated copy, each with its id, and records it in the
/// ledger. Returns the plan and the keys written, whose `dst_hash` the caller refreshes with
/// `settle` after its own reindex, which can still give a copy a new id when another note holds
/// its id.
pub fn apply(
    src: &Vault,
    memory: Option<&Path>,
    dst: &Vault,
    ledger: &mut Ledger,
    w: &Written<'_>,
) -> anyhow::Result<(ImportPlan, Vec<String>)> {
    let mut plan = ImportPlan::default();
    let all = sources(src, memory, dst, &mut plan.skipped)?;
    // A record from before the ledger kept where a source was found learns it here.
    for s in &all {
        let present = dst.root.join(&s.dst_rel).is_file();
        if let Some(entry) = ledger.0.get_mut(&s.key)
            && entry.of(&dst.root)
            && present
        {
            entry.from = entry.from.take().or_else(|| s.from.clone());
            entry.vault = Some(dst.root.clone());
        }
    }
    let todo = classify_sources(all, dst, ledger, &mut plan);
    let mut written = Vec::new();
    for (s, seen) in todo {
        write_one(&s, seen, None, dst, ledger, w)?;
        written.push(s.key);
    }
    Ok((plan, written))
}

/// Writes one source's copy and records it. `returning` is the id its copy had when it was
/// archived, for a source that has no record because of that.
fn write_one(
    s: &Source,
    seen: Option<String>,
    returning: Option<String>,
    dst: &Vault,
    ledger: &mut Ledger,
    w: &Written<'_>,
) -> anyhow::Result<()> {
    let prior = ledger
        .current(&s.key, &dst.root)
        .and_then(|e| e.id.clone())
        .or(returning);
    let (text, id) = if s.memory {
        let id = prior.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
        (memory::to_note(s, &id, w)?, Some(id))
    } else {
        let text = vault::to_note(s, w, prior.as_deref())?;
        let id = brain_note::Note::parse(&s.dst_rel, &text, brain_note::Place::Kept)
            .id
            .map(|u| u.to_string());
        (text, id)
    };
    // The copy as classified: absent, or the bytes the ledger recorded. A save made to it since
    // is left alone.
    replace_unchanged(
        &dst.root,
        &s.dst_rel,
        |now| now.map(hash) == seen,
        text.as_bytes(),
        Privacy::Normal,
    )
    .with_context(|| format!("writing {}", rel_key(&s.dst_rel)))?;
    // Best effort: a copy that could not be stamped says it changed when brain wrote it.
    if let Some(at) = s.modified {
        let _ = set_modified(&dst.root, &s.dst_rel, at);
    }
    ledger.0.insert(
        s.key.clone(),
        LedgerEntry {
            id,
            src_hash: hash(s.raw.as_bytes()),
            dst_hash: hash(text.as_bytes()),
            from: s.from.clone(),
            vault: Some(dst.root.clone()),
            missing: false,
        },
    );
    Ok(())
}

/// Records each written copy's bytes as they are now, after the index rewrote any id, so
/// brain's own write never reads as the person's edit on the next import.
pub fn settle(ledger: &mut Ledger, dst: &Vault, written: &[String]) -> anyhow::Result<()> {
    for key in written {
        let Some(entry) = ledger.0.get_mut(key) else {
            continue;
        };
        let rel = match key.split_once('/') {
            Some(("vault", rel)) => PathBuf::from(rel),
            Some(("memory", rest)) => dst.layout.memory.join(rest),
            _ => continue,
        };
        if let Ok(bytes) = std::fs::read(dst.root.join(&rel)) {
            entry.dst_hash = hash(&bytes);
            let text = String::from_utf8_lossy(&bytes);
            if let Some(id) = brain_note::Note::parse(&rel, &text, brain_note::Place::Kept).id {
                entry.id = Some(id.to_string());
            }
        }
    }
    Ok(())
}
