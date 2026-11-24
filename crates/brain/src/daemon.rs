//! `brain daemon`: the index's one writer. It holds the writer lock for as long as it runs,
//! watches the vault and brings the index up to date after any change that can matter, so a
//! saved note is searchable seconds later. `brain index` and `brain import` defer to it.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use brain_index::{Index, Reindexed, embed};
use brain_note::Vault;
use brain_platform::clock::Clock;
use brain_platform::dirs::Dirs;
use brain_platform::fs::Lock;
use notify::event::ModifyKind;
use notify::{EventKind, EventKindMask, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, NoCache};

use crate::config::Config;
use crate::out::terminal;

pub const WRITER_LOCK: &str = "writer.lock";
/// A full pass this long after the last, for whatever the watcher missed.
const BACKSTOP: Duration = Duration::from_secs(600);
/// A note saved this recently gets no id written into it yet: its owner may still be typing.
const SETTLE: Duration = Duration::from_secs(5);
/// When a pass left a young note without its id, the next pass comes this soon.
const RETRY: Duration = Duration::from_secs(6);
const EMBED_BATCH: usize = 32;

pub fn writer_lock(dirs: &Dirs) -> PathBuf {
    dirs.data.join(WRITER_LOCK)
}

/// The line a pass prints, and `brain index` too.
pub fn summary(r: &Reindexed) -> String {
    let mut line = format!(
        "{} added, {} changed, {} removed, {} skipped, {} ids written, {} without an id",
        r.added,
        r.changed,
        r.removed,
        r.skipped.len(),
        r.ids_written,
        r.unidentified.len()
    );
    if r.deferred > 0 {
        line.push_str(&format!(
            ", {} left until their owner stops typing",
            r.deferred
        ));
    }
    line
}

pub fn run(dirs: &Dirs, clock: &dyn Clock) -> anyhow::Result<()> {
    let path = writer_lock(dirs);
    let _lock = match Lock::try_acquire(&path)? {
        Some(lock) => lock,
        None => {
            eprintln!("waiting for another brain writer to finish");
            Lock::acquire(&path)?
        }
    };
    let vault = Vault::open(&Config::load(dirs)?.vault)?;
    let mut idx = Index::open(&dirs.data.join("index.db"))?;
    let provider = embed::load()
        .map_err(|e| {
            eprintln!(
                "{}",
                terminal(&format!(
                    "embeddings off, search uses words and links: {e:#}"
                ))
            )
        })
        .ok();
    let (tx, rx) = mpsc::channel();
    // The default file-id cache walks the tree following links; the daemon needs no rename
    // pairing, so it keeps none, and the watcher itself follows no link, as the vault walk does not.
    let config = notify_debouncer_full::Config::default().with_notify_config(
        notify::Config::default()
            .with_event_kinds(EventKindMask::CORE)
            .with_follow_symlinks(false),
    );
    let mut watcher = notify_debouncer_full::new_debouncer_opt::<_, notify::RecommendedWatcher, _>(
        config, tx, NoCache,
    )?;
    watcher.watch(&vault.root, RecursiveMode::Recursive)?;
    let mut passes = 0u64;
    let mut due = true;
    let mut embedding = false;
    let mut next = Instant::now();
    loop {
        if due {
            passes += 1;
            match clock
                .now()
                .checked_sub(SETTLE)
                .map_err(anyhow::Error::from)
                .and_then(|settled| idx.reindex_settled(&vault, settled))
            {
                Ok(r) => {
                    eprintln!("{}", terminal(&format!("pass {passes}: {}", summary(&r))));
                    next = Instant::now() + if r.deferred > 0 { RETRY } else { BACKSTOP };
                }
                Err(e) => {
                    eprintln!("{}", terminal(&format!("pass {passes} failed: {e:#}")));
                    next = Instant::now() + BACKSTOP;
                }
            }
            embedding = provider.is_some();
        }
        // One batch a turn, so a save made during a long first embed waits for one batch only.
        if embedding && let Some(p) = &provider {
            embedding = match idx.embed_batch(&**p, EMBED_BATCH) {
                Ok(e) => {
                    if let Some(why) = &e.stopped {
                        eprintln!("{}", terminal(&format!("embedding stopped: {why}")));
                    }
                    e.pending > 0 && e.stopped.is_none()
                }
                Err(e) => {
                    eprintln!("{}", terminal(&format!("embedding failed: {e:#}")));
                    false
                }
            };
        }
        let wait = if embedding {
            Duration::ZERO
        } else {
            next.saturating_duration_since(Instant::now())
        };
        due = match rx.recv_timeout(wait) {
            Ok(event) => matters(&vault.root, &event),
            Err(RecvTimeoutError::Timeout) => Instant::now() >= next,
            Err(RecvTimeoutError::Disconnected) => anyhow::bail!("the vault watcher stopped"),
        };
        while let Ok(event) = rx.try_recv() {
            due |= matters(&vault.root, &event);
        }
    }
}

/// Whether a batch of watcher events can change the index. The exact events an editor's save
/// makes differ by system and editor, so any `.md` path counts, and so does any rename or
/// removal, which can take a folder of notes with it; nothing inside a hidden folder does.
fn matters(root: &Path, result: &DebounceEventResult) -> bool {
    let Ok(events) = result else {
        return true;
    };
    events.iter().any(|e| {
        e.need_rescan()
            || e.paths.iter().any(|p| {
                let Ok(rel) = p.strip_prefix(root) else {
                    return true;
                };
                let hidden = rel.components().any(
                    |c| matches!(c, Component::Normal(n) if n.to_string_lossy().starts_with('.')),
                );
                !hidden
                    && (p.extension().is_some_and(|x| x == "md")
                        || matches!(
                            e.kind,
                            EventKind::Modify(ModifyKind::Name(_)) | EventKind::Remove(_)
                        ))
            })
    })
}
