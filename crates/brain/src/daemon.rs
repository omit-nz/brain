//! `brain daemon`: the index's one writer. It holds the writer lock for as long as it runs,
//! watches the vault and brings the index up to date after any change that can matter, so a
//! saved note is searchable seconds later. `brain index` and `brain import` defer to it.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use brain::update;
use brain_index::activity::Activity;
use brain_index::{Index, Reindexed, embed};
use brain_note::Vault;
use brain_platform::clock::Clock;
use brain_platform::dirs::Dirs;
use brain_platform::fs::Lock;
use brain_platform::process;
use brain_report::Event;
use ll_core::embed::EmbeddingProvider;
use notify::event::ModifyKind;
use notify::{EventKind, EventKindMask, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, NoCache};

use crate::config::Config;
use crate::import::{self, Followed, Written};
use crate::out::terminal;
use crate::{gaps, plugin, usage};

pub const WRITER_LOCK: &str = "writer.lock";
/// A full pass this long after the last, for whatever the watcher missed.
const BACKSTOP: Duration = Duration::from_secs(600);
/// A note saved this recently gets no id written into it yet: its owner may still be typing.
const SETTLE: Duration = Duration::from_secs(5);
/// When a pass left a young note without its id, the next pass comes this soon.
const RETRY: Duration = Duration::from_secs(6);
const EMBED_BATCH: usize = 32;
const RECHECK_EVERY: Duration = Duration::from_secs(3600);
const SEND_EVERY: Duration = Duration::from_secs(3600);
const FOLLOW_EVERY: Duration = Duration::from_secs(60);
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);

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
            ", {} waiting for their owner to stop typing to get an id",
            r.deferred
        ));
    }
    line
}

/// How often a followed memory folder is looked at; a debug build lets a test hurry it.
fn follow_every() -> Duration {
    let hurried = cfg!(debug_assertions)
        .then(|| std::env::var("BRAIN_TEST_FOLLOW_EVERY_MS").ok())
        .flatten();
    hurried
        .and_then(|ms| ms.parse().ok())
        .map_or(FOLLOW_EVERY, Duration::from_millis)
}

/// One pass over the followed auto-memory, when one is followed and no other command holds the
/// import ledger. The config is read each time, so following starts without a restart.
fn follow_memory(
    dirs: &Dirs,
    idx: &mut Index,
    clock: &dyn Clock,
) -> anyhow::Result<Option<Followed>> {
    let config = Config::load(dirs)?;
    let Some(followed) = &config.memory.follow else {
        return Ok(None);
    };
    let Some(_held) = Lock::try_acquire(&dirs.data.join(import::LOCK))? else {
        return Ok(None);
    };
    let vault = Vault::open(&config.vault)?;
    let w = Written::for_memory(&config.me, clock.today(), &vault);
    let done = import::sync(dirs, &vault, followed, &w, |vault| {
        let r = idx.reindex(vault)?;
        forget(dirs, &r);
        Ok(())
    })?;
    Ok(Some(done))
}

/// Drops what traces and verdicts kept of the notes a pass saw go. A failure is said and no
/// more: both stores also prune by age.
pub fn forget(dirs: &Dirs, r: &Reindexed) {
    let sources: Vec<String> = r.gone.iter().map(|id| format!("note:{id}")).collect();
    if let Err(e) = brain_reflex::purge(&dirs.data, &sources) {
        eprintln!(
            "{}",
            terminal(&format!(
                "purging removed notes from the traces failed: {e:#}"
            ))
        );
    }
}

/// Installs a newer signed release over the install this daemon runs from, through the same
/// checks `brain update` makes, and returns its version. A failure leaves this version running; a
/// brain run from anywhere but its install (a dev build) never replaces anything.
fn update_self(
    dirs: &Dirs,
    store: &Path,
    clock: &dyn Clock,
    from_dir: Option<&Path>,
) -> Option<String> {
    let current = update::install_dir(&dirs.bin);
    if !update::runs_from(&current) {
        eprintln!("update: this brain was not run from its install, so it never updates itself");
        return None;
    }
    let version = env!("CARGO_PKG_VERSION");
    let installed = match from_dir {
        Some(dir) => newer(|| update::LocalDir(dir.to_path_buf()), &current, version),
        None => newer(|| update::GitHub, &current, version),
    };
    installed
        .map_err(|e| {
            eprintln!("{}", terminal(&format!("update failed: {e:#}")));
            usage::count(store, clock.today(), Event::ErrorUpdate, None);
        })
        .ok()
        .flatten()
}

/// The version installed over `current`, when a newer one was out.
fn newer<S: update::Source>(
    source: impl Fn() -> S,
    current: &Path,
    version: &str,
) -> anyhow::Result<Option<String>> {
    if update::check(source(), version)?.is_none() {
        return Ok(None);
    }
    let v = update::run(source(), &update::release_trust(), current, version)?;
    Ok((v != version).then_some(v))
}

/// Runs the daemon until it installs a newer release, then runs that release in its place with
/// the same arguments: the same process under launchd, and the same terminal elsewhere.
pub fn run(dirs: &Dirs, clock: Arc<dyn Clock>, from_dir: Option<&Path>) -> anyhow::Result<()> {
    // An update restarts the daemon, which is when an installed plugin learns the new release.
    if let Err(e) = plugin::refresh(dirs) {
        eprintln!(
            "{}",
            terminal(&format!("the Claude Code plugin was not updated: {e:#}"))
        );
    }
    let v = serve(dirs, &clock, from_dir)?;
    eprintln!("installed brain {v}; starting it");
    let exe = update::install_dir(&dirs.bin).join(update::exe_name());
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    Err(anyhow::Error::new(process::replace(&exe, &args))
        .context(format!("starting {}", exe.display())))
}

/// The daemon's work, holding the writer lock and the vault watcher, which it lets go of when it
/// returns the version it installed.
fn serve(dirs: &Dirs, clock: &Arc<dyn Clock>, from_dir: Option<&Path>) -> anyhow::Result<String> {
    let path = writer_lock(dirs);
    let _lock = match Lock::try_acquire(&path)? {
        Some(lock) => lock,
        None => {
            eprintln!("waiting for another brain writer to finish");
            Lock::acquire(&path)?
        }
    };
    // A vault on a disk not yet mounted at login is waited for, not exited on, so launchd never
    // restarts the daemon into the same failure every few seconds.
    let root = Config::load(dirs)?.vault;
    let mut said = false;
    while !root.exists() {
        if !said {
            eprintln!(
                "{}",
                terminal(&format!(
                    "no vault at {} yet; waiting for it",
                    root.display()
                ))
            );
            said = true;
        }
        std::thread::sleep(RETRY);
    }
    let vault = Vault::open(&root)?;
    let index_db = dirs.data.join("index.db");
    let mut idx = Index::open(&index_db)?;
    let provider = embed::load()
        .map_err(|e| {
            eprintln!(
                "{}",
                terminal(&format!(
                    "embeddings off, search uses words and links: {e:#}"
                ))
            )
        })
        .ok()
        .map(Arc::<dyn EmbeddingProvider>::from);
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
    let store = usage::store(dirs);
    let mut passes = 0u64;
    let mut due = true;
    let mut embedding = false;
    let mut rewatch = false;
    let mut changed = false;
    let mut last_recheck: Option<Instant> = None;
    let mut rechecking: Option<JoinHandle<()>> = None;
    let mut last_send: Option<Instant> = None;
    let mut last_update: Option<Instant> = None;
    let mut last_follow: Option<Instant> = None;
    let mut follow_said = None;
    let mut next = Instant::now();
    loop {
        if due {
            passes += 1;
            if rewatch && watcher.watch(&vault.root, RecursiveMode::Recursive).is_ok() {
                rewatch = false;
            }
            match clock
                .now()
                .checked_sub(SETTLE)
                .map_err(anyhow::Error::from)
                .and_then(|settled| idx.reindex_settled(&vault, settled, clock.now()))
            {
                Ok(r) => {
                    forget(dirs, &r);
                    eprintln!("{}", terminal(&format!("pass {passes}: {}", summary(&r))));
                    next = Instant::now() + if r.deferred > 0 { RETRY } else { BACKSTOP };
                    changed |= r.added + r.changed + r.removed > 0;
                }
                Err(e) => {
                    eprintln!("{}", terminal(&format!("pass {passes} failed: {e:#}")));
                    usage::count(&store, clock.today(), Event::ErrorIndex, None);
                    next = Instant::now() + RETRY;
                    // A vault folder that went away and came back is a new folder to the
                    // watcher on some systems; watching it again costs nothing when it is not.
                    let _ = watcher.unwatch(&vault.root);
                    rewatch = true;
                }
            }
            embedding = provider.is_some();
            // At start and hourly; a pass comes at least every BACKSTOP, so the hour holds.
            if last_send.is_none_or(|t: Instant| t.elapsed() >= SEND_EVERY) {
                last_send = Some(Instant::now());
                if let Err(e) = brain_reflex::prune(&dirs.data, clock.now()) {
                    eprintln!("{}", terminal(&format!("pruning traces failed: {e:#}")));
                }
                match usage::send_due(dirs, &idx, &**clock) {
                    Ok(s) => eprintln!("{}", s.line()),
                    Err(e) => eprintln!("{}", terminal(&format!("usage: sending failed: {e:#}"))),
                }
            }
            // Not while a recheck runs: off Unix the old process waits on the new one, and the
            // recheck would run on beside it.
            if rechecking.as_ref().is_none_or(JoinHandle::is_finished)
                && last_update.is_none_or(|t: Instant| t.elapsed() >= UPDATE_EVERY)
            {
                last_update = Some(Instant::now());
                if let Some(v) = update_self(dirs, &store, &**clock, from_dir) {
                    return Ok(v);
                }
            }
        }
        if last_follow.is_none_or(|t: Instant| t.elapsed() >= follow_every()) {
            last_follow = Some(Instant::now());
            let said = match follow_memory(dirs, &mut idx, &**clock) {
                Ok(Some(done)) => {
                    changed |= done.changed();
                    embedding |= done.changed() && provider.is_some();
                    (done.changed() || !done.skipped.is_empty())
                        .then(|| format!("memory: {}", done.summary()))
                }
                Ok(None) => None,
                Err(e) => Some(format!("memory: {e:#}")),
            };
            // A folder that stays away is said once, not every minute.
            if said.is_some() && said != follow_said {
                eprintln!("{}", terminal(said.as_deref().unwrap_or_default()));
            }
            follow_said = said;
        }
        // One batch a turn, so a save made during a long first embed waits for one batch only.
        if embedding && let Some(p) = &provider {
            embedding = match idx.embed_batch(&**p, EMBED_BATCH) {
                Ok(e) => {
                    if let Some(why) = &e.stopped {
                        eprintln!("{}", terminal(&format!("embedding stopped: {why}")));
                        usage::count(&store, clock.today(), Event::ErrorEmbed, None);
                    }
                    e.pending > 0 && e.stopped.is_none()
                }
                Err(e) => {
                    eprintln!("{}", terminal(&format!("embedding failed: {e:#}")));
                    usage::count(&store, clock.today(), Event::ErrorEmbed, None);
                    false
                }
            };
        }
        // Each recheck searches every open gap, so it waits for a pass that changed notes and for
        // their vectors, and runs at most once an hour, never once per save.
        if changed
            && !embedding
            && last_recheck.is_none_or(|t: Instant| t.elapsed() >= RECHECK_EVERY)
            && rechecking.as_ref().is_none_or(JoinHandle::is_finished)
        {
            changed = false;
            last_recheck = Some(Instant::now());
            rechecking = Some(recheck(&index_db, &store, provider.clone(), clock.clone()));
        }
        let wait = if embedding {
            Duration::ZERO
        } else {
            let followed = last_follow.map(|t| follow_every().saturating_sub(t.elapsed()));
            next.saturating_duration_since(Instant::now())
                .min(followed.unwrap_or_default())
        };
        let mut events = Vec::new();
        match rx.recv_timeout(wait) {
            Ok(event) => events.push(event),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => anyhow::bail!("the vault watcher stopped"),
        }
        events.extend(rx.try_iter());
        due = Instant::now() >= next;
        for event in &events {
            if event.is_err() {
                usage::count(&store, clock.today(), Event::ErrorWatch, None);
            }
            due |= matters(&vault.root, event);
        }
    }
}

/// Searches the open gaps again beside the daemon's loop, on its own read of the index: each is a
/// reranked search, and a few hundred in turn would keep every save waiting for minutes.
fn recheck(
    index_db: &Path,
    store: &Path,
    provider: Option<Arc<dyn EmbeddingProvider>>,
    clock: Arc<dyn Clock>,
) -> JoinHandle<()> {
    let (index_db, store) = (index_db.to_path_buf(), store.to_path_buf());
    std::thread::spawn(move || {
        let filled = Index::open(&index_db).and_then(|idx| {
            let activity = Activity::open(&store)?;
            gaps::recheck(&idx, &activity, provider.as_deref(), &*clock)
        });
        match filled {
            Ok(n) => eprintln!("recheck: {n} gaps filled"),
            Err(e) => eprintln!("{}", terminal(&format!("recheck failed: {e:#}"))),
        }
    })
}

/// Whether a batch of watcher events can change the index. The exact events an editor's save
/// makes differ by system and editor, so any `.md` path counts, and so does any creation, rename
/// or removal, which can bring or take a folder of notes (Windows reports a folder moved in as
/// one creation); nothing inside a hidden folder does.
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
                            EventKind::Create(_)
                                | EventKind::Modify(ModifyKind::Name(_))
                                | EventKind::Remove(_)
                        ))
            })
    })
}
