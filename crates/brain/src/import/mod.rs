//! Importing a learning-loop brain: its vault and its auto-memory, copied into the brain vault.
//! The source is only read. Every note comes across private unless it declares otherwise or sits
//! in a folder the person chose to share and learning-loop's own rules do not withhold it.
//! A ledger remembers what was copied, so a second import updates only what changed and never
//! overwrites a copy the person edited.

mod blocklist;
mod memory;
mod report;
mod vault;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use brain_note::{Skip, Vault, rel_key};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, write_atomic};
use jiff::ToSpan;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use blocklist::Blocklist;
pub use report::report;

/// Days over which first reviews are spread.
const REVIEW_SPREAD: i64 = 90;
const LEDGER: &str = "import-ledger.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: Option<String>,
    pub src_hash: String,
    pub dst_hash: String,
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
}

/// One file the import reads and where its copy goes.
pub struct Source {
    key: String,
    dst_rel: PathBuf,
    raw: String,
    memory: bool,
}

/// What every copy is written with.
pub struct Written<'a> {
    pub me: &'a str,
    pub today: Date,
    pub share: &'a [PathBuf],
    pub blocklist: Option<&'a Blocklist>,
    pub inbox: &'a Path,
}

impl Written<'_> {
    /// Visibility for a note that declares none: a learning-loop rule that withholds it wins,
    /// then a shared folder; everything else is private.
    fn visibility(&self, rel: &Path) -> &'static str {
        if self.blocklist.is_some_and(|b| b.withholds(&rel_key(rel))) {
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
    skipped: &mut Vec<(String, Skip)>,
) -> anyhow::Result<Vec<Source>> {
    let mut out = vault::sources(src, skipped);
    if let Some(root) = memory {
        out.extend(memory::sources(root, skipped)?);
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
) -> anyhow::Result<(ImportPlan, Vec<Source>)> {
    let mut plan = ImportPlan::default();
    let all = sources(src, memory, &mut plan.skipped)?;
    let mut todo = Vec::new();
    for s in all {
        let src_hash = hash(s.raw.as_bytes());
        let dst_path = dst.root.join(&s.dst_rel);
        let dst_hash = std::fs::read(&dst_path).ok().map(|b| hash(&b));
        match ledger.0.get(&s.key) {
            None if dst_hash.is_some() => plan.conflicts.push(rel_key(&s.dst_rel)),
            None => {
                plan.new.push(rel_key(&s.dst_rel));
                todo.push(s);
            }
            Some(e) if e.src_hash == src_hash => plan.unchanged += 1,
            Some(e) if dst_hash.as_deref() == Some(e.dst_hash.as_str()) => {
                plan.updated.push(rel_key(&s.dst_rel));
                todo.push(s);
            }
            Some(_) => plan.conflicts.push(rel_key(&s.dst_rel)),
        }
    }
    Ok((plan, todo))
}

pub fn plan(
    src: &Vault,
    memory: Option<&Path>,
    dst: &Vault,
    ledger: &Ledger,
) -> anyhow::Result<ImportPlan> {
    Ok(classify(src, memory, dst, ledger)?.0)
}

/// Classifies, then writes every new and updated copy and records it in the ledger. Returns the
/// plan and the keys written, whose `dst_hash` the caller refreshes with `settle` once the index
/// has written ids into the copies.
pub fn apply(
    src: &Vault,
    memory: Option<&Path>,
    dst: &Vault,
    ledger: &mut Ledger,
    w: &Written<'_>,
) -> anyhow::Result<(ImportPlan, Vec<String>)> {
    let (plan, todo) = classify(src, memory, dst, ledger)?;
    let mut written = Vec::new();
    for s in todo {
        let prior = ledger.0.get(&s.key).and_then(|e| e.id.clone());
        let (text, id) = if s.memory {
            let id = prior.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
            (memory::to_note(&s, &id, w)?, Some(id))
        } else {
            let text = vault::to_note(&s, w, prior.as_deref())?;
            let id = brain_note::Note::parse(&s.dst_rel, &text, brain_note::Place::Kept)
                .id
                .map(|u| u.to_string());
            (text, id)
        };
        write_atomic(&dst.root, &s.dst_rel, text.as_bytes(), Privacy::Normal)
            .with_context(|| format!("writing {}", rel_key(&s.dst_rel)))?;
        ledger.0.insert(
            s.key.clone(),
            LedgerEntry {
                id,
                src_hash: hash(s.raw.as_bytes()),
                dst_hash: hash(text.as_bytes()),
            },
        );
        written.push(s.key);
    }
    Ok((plan, written))
}

/// Records each written copy's bytes as they are now, after the index wrote any missing id, so
/// brain's own write never reads as the person's edit on the next import.
pub fn settle(ledger: &mut Ledger, dst: &Vault, written: &[String]) -> anyhow::Result<()> {
    for key in written {
        let Some(entry) = ledger.0.get_mut(key) else {
            continue;
        };
        let rel = match key.split_once('/') {
            Some(("vault", rel)) => PathBuf::from(rel),
            Some(("memory", rest)) => Path::new("working-memory").join(rest),
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
