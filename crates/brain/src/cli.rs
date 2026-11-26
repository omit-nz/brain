use std::path::{Path, PathBuf};

use anyhow::Context;
use brain_index::activity::Activity;
use brain_index::{Index, Query, Scope, embed, is_gap};
use brain_note::{Kind, Vault};
use brain_platform::clock::{Clock, FixedClock, SystemClock};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Lock, create_dir_private};
use brain_platform::service::{self, Launchctl};
use brain_report::{Door, Event};
use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};
use jiff::ToSpan;
use serde_json::json;

use brain::update;

use crate::capture::{self, Capture};
use crate::config::Config;
use crate::connect;
use crate::daemon::{self, writer_lock};
use crate::gaps;
use crate::import::{self, Blocklist, Ledger, Written};
use crate::inbox::{self, Dest};
use crate::mcp;
use crate::out::terminal;
use crate::usage;

#[derive(Parser)]
#[command(
    name = "brain",
    version = concat!(env!("CARGO_PKG_VERSION"), " (protocol 1)"),
    about = "A team's notes, searchable on your own machine."
)]
pub struct Cli {
    /// Read the clock at this instant instead of the system's (for tests).
    #[arg(long, global = true, hide = true)]
    now: Option<jiff::Timestamp>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set brain up over a folder of notes (the current folder unless --vault names one).
    Init {
        #[arg(long)]
        me: String,
        #[arg(long)]
        vault: Option<PathBuf>,
    },
    /// Bring the index up to date with the vault, and embed what is new.
    Index,
    /// Keep the index up to date as notes change, until stopped.
    Daemon {
        /// Read releases from this folder instead of GitHub (for tests).
        #[arg(long, hide = true)]
        from_dir: Option<PathBuf>,
    },
    /// Search your notes.
    Search {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[command(flatten)]
        s: SearchArgs,
    },
    /// Questions no note answered yet, newest first.
    Gaps {
        #[arg(long)]
        json: bool,
    },
    /// Write a new note into the inbox.
    Add {
        text: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Write a rule note that corrects another note, which is left as it is.
    Correct {
        /// The note's id or its path in the vault.
        target: String,
        text: String,
    },
    /// What brain counts about its own use, and whether the day's counts go to omit.
    Usage {
        #[command(subcommand)]
        action: Option<UsageAction>,
    },
    /// Serve brain to an AI tool over MCP on stdin and stdout.
    Mcp {
        /// The AI tool this server answers, as its counts name it.
        #[arg(long, value_enum, default_value = "other")]
        door: DoorArg,
        /// Hand over to the installed brain when this copy is not it (the Claude Desktop
        /// extension's bundled copy).
        #[arg(long, hide = true)]
        hand_over: bool,
    },
    /// Connect an AI tool to the installed brain.
    Connect {
        #[arg(value_enum)]
        tool: ConnectTool,
    },
    /// Your kept notes whose review has come, oldest first.
    Due {
        /// Also list notes due within this many days.
        #[arg(long, default_value_t = 14)]
        within: i64,
        #[arg(long)]
        json: bool,
    },
    /// Print a note, by id or by its path in the vault.
    Show { target: String },
    /// Copy another brain's notes into this vault.
    Import {
        #[command(subcommand)]
        from: ImportFrom,
    },
    /// List the inbox, oldest first, or keep or skip a note in it.
    Inbox {
        #[command(subcommand)]
        action: Option<InboxAction>,
    },
    /// What the index holds.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Install the newest signed release over this one.
    Update {
        /// Only say whether a newer release exists.
        #[arg(long)]
        check: bool,
        /// Read releases from this folder instead of GitHub (for tests).
        #[arg(long, hide = true)]
        from_dir: Option<PathBuf>,
    },
    /// Run the daemon at every login, and now (macOS).
    Start,
    /// Stop the daemon, and keep it from starting at login.
    Stop,
    /// Check a release archive's signature against this build's release key.
    #[command(hide = true)]
    ReleaseVerify { archive: PathBuf },
    /// Print the folder an install lives in, for scripts/install.sh.
    #[command(hide = true)]
    InstallDir,
}

#[derive(Args)]
struct SearchArgs {
    #[arg(long, default_value_t = 10)]
    limit: usize,
    #[arg(long)]
    json: bool,
    /// Only what a colleague could see.
    #[arg(long)]
    shared_only: bool,
}

#[derive(Subcommand)]
enum ImportFrom {
    /// A learning-loop vault and, optionally, Claude Code auto-memory.
    LearningLoop(LearningLoop),
}

#[derive(Args)]
#[command(group(ArgGroup::new("blocklist").args(["ll_config", "no_ll_config"])))]
struct LearningLoop {
    /// The learning-loop vault; it is only read.
    #[arg(long)]
    from: PathBuf,
    /// Claude Code's projects folder, whose `*/memory/*.md` become working memory.
    #[arg(long)]
    memory: Option<PathBuf>,
    /// A vault folder whose undeclared notes come across shared.
    #[arg(long, requires = "blocklist")]
    share: Vec<PathBuf>,
    /// learning-loop's config folder, whose visibility rules can keep a note private.
    #[arg(long)]
    ll_config: Option<PathBuf>,
    /// Share without learning-loop's rules.
    #[arg(long)]
    no_ll_config: bool,
    /// Show what would change and write nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum InboxAction {
    /// Move a note out of the inbox, unchanged.
    Keep {
        /// The note's id or its path in the vault.
        target: String,
        #[arg(long, value_enum, default_value = "fleeting")]
        to: Dest,
    },
    /// Archive a note; nothing is deleted.
    Skip {
        /// The note's id or its path in the vault.
        target: String,
    },
}

#[derive(Subcommand)]
enum UsageAction {
    /// Send the day's counts to omit (the default).
    On,
    /// Keep the counts on this machine only.
    Off,
    /// Send the closed days' counts now; the daemon does this hourly.
    #[command(hide = true)]
    Send,
}

#[derive(Clone, Copy, ValueEnum)]
enum ConnectTool {
    /// Claude Code, through `claude mcp add`.
    ClaudeCode,
}

#[derive(Clone, Copy, ValueEnum)]
enum DoorArg {
    ClaudeCode,
    ClaudeDesktop,
    Other,
}

impl From<DoorArg> for Door {
    fn from(d: DoorArg) -> Door {
        match d {
            DoorArg::ClaudeCode => Door::ClaudeCode,
            DoorArg::ClaudeDesktop => Door::ClaudeDesktop,
            DoorArg::Other => Door::Other,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    Knowledge,
    Rule,
    History,
}

impl From<KindArg> for Kind {
    fn from(k: KindArg) -> Kind {
        match k {
            KindArg::Knowledge => Kind::Knowledge,
            KindArg::Rule => Kind::Rule,
            KindArg::History => Kind::History,
        }
    }
}

fn index(dirs: &Dirs) -> anyhow::Result<Index> {
    Index::open(&dirs.data.join("index.db"))
}

pub fn run(cli: Cli, dirs: &Dirs) -> anyhow::Result<()> {
    let clock: Box<dyn Clock> = match cli.now {
        Some(t) => Box::new(FixedClock(t)),
        None => Box::new(SystemClock),
    };
    match cli.command {
        Command::Init { me, vault } => init(dirs, me, vault),
        Command::Index => reindex(dirs, &*clock),
        Command::Daemon { from_dir } => daemon::run(dirs, &*clock, from_dir.as_deref()),
        Command::Search { text, s } => search(dirs, &*clock, &text.join(" "), s),
        Command::Gaps { json } => list_gaps(dirs, &*clock, json),
        Command::Add { text, title, kind } => {
            let config = Config::load(dirs)?;
            let vault = Vault::open(&config.vault)?;
            let c = Capture {
                text: &text,
                title: title.as_deref(),
                kind: kind.map(Kind::from),
                corrects: None,
            };
            let w = capture::write(&vault, &config.me, clock.today(), &c)?;
            println!("{}", terminal(&w.rel));
            let cli = Some(Door::Cli);
            usage::count(
                &usage::store(dirs),
                clock.today(),
                Event::CaptureWritten,
                cli,
            );
            Ok(())
        }
        Command::Correct { target, text } => {
            let config = Config::load(dirs)?;
            let vault = Vault::open(&config.vault)?;
            let today = clock.today();
            let w = capture::correct(&vault, &index(dirs)?, &config.me, today, &target, &text)?;
            println!("{}", terminal(&w.rel));
            usage::count(
                &usage::store(dirs),
                today,
                Event::Correction,
                Some(Door::Cli),
            );
            Ok(())
        }
        Command::Usage { action } => match action {
            None => usage::show(dirs, clock.today()),
            Some(UsageAction::On) => usage::set(dirs, &*clock, true),
            Some(UsageAction::Off) => usage::set(dirs, &*clock, false),
            Some(UsageAction::Send) => {
                println!("{}", usage::send_due(dirs, &index(dirs)?, &*clock)?.line());
                Ok(())
            }
        },
        Command::Due { within, json } => due(dirs, &*clock, within, json),
        Command::Mcp { door, hand_over } => {
            if hand_over {
                let name = door.to_possible_value().expect("every door has a name");
                mcp::hand_over(dirs, name.get_name())?;
            }
            mcp::run(dirs, clock, door.into())
        }
        Command::Connect {
            tool: ConnectTool::ClaudeCode,
        } => connect::claude_code(dirs),
        Command::Show { target } => show(dirs, &target),
        Command::Status { json } => status(dirs, json),
        Command::Update { check, from_dir } => match from_dir {
            Some(dir) => update(dirs, check, update::LocalDir(dir)),
            None => update(dirs, check, update::GitHub),
        },
        Command::Start => start(dirs),
        Command::Stop => {
            launchctl().stop()?;
            println!("brain's daemon is stopped and will not start at login");
            Ok(())
        }
        Command::ReleaseVerify { archive } => {
            update::verify(&archive, &update::release_trust().keys)?;
            println!(
                "{}: signed by the brain release key",
                terminal(&archive.display().to_string())
            );
            Ok(())
        }
        Command::InstallDir => {
            println!("{}", update::install_dir(&dirs.bin).display());
            Ok(())
        }
        Command::Import {
            from: ImportFrom::LearningLoop(args),
        } => import_learning_loop(dirs, clock.today(), args),
        Command::Inbox { action } => {
            let vault = Vault::open(&Config::load(dirs)?.vault)?;
            let (result, event) = match action {
                None => return inbox::list(&vault),
                Some(InboxAction::Keep { target, to }) => {
                    (inbox::keep(&vault, &target, to), Event::CaptureKept)
                }
                Some(InboxAction::Skip { target }) => {
                    (inbox::skip(&vault, &target), Event::CaptureSkipped)
                }
            };
            result?;
            usage::count(&usage::store(dirs), clock.today(), event, Some(Door::Cli));
            Ok(())
        }
    }
}

fn init(dirs: &Dirs, me: String, vault: Option<PathBuf>) -> anyhow::Result<()> {
    let named = vault.is_some();
    let root = match vault {
        Some(v) => v,
        None => std::env::current_dir()?,
    };
    // brain writes an id into every note under its vault, so a folder holding far more than notes
    // is refused. A git repository is refused only as the folder init happens to run in: a notes
    // folder synced with git is a vault once its owner names it.
    let same = |a: &Path, b: &Path| {
        a.canonicalize()
            .ok()
            .is_some_and(|a| Some(a) == b.canonicalize().ok())
    };
    let what = if std::env::home_dir().is_some_and(|home| same(&root, &home)) {
        Some("your home folder")
    } else if !named && root.join(".git").exists() {
        Some("a code repository")
    } else if root.canonicalize().is_ok_and(|r| r.parent().is_none()) {
        Some("the root of the disk")
    } else {
        None
    };
    if let Some(what) = what {
        anyhow::bail!(
            "{} is {what}; brain writes an id into every note in its vault, so name your notes folder with --vault",
            root.display()
        );
    }
    if !root.exists() {
        create_dir_private(&root)?;
    }
    let vault = Vault::open(&root)?;
    let layout = &vault.layout;
    for folder in [
        &layout.inbox,
        &layout.private,
        &layout.memory,
        &layout.archive,
    ] {
        let path = vault.root.join(folder);
        if !path.exists() {
            create_dir_private(&path)?;
        }
    }
    // A second init keeps the person's usage choice; an unreadable config fails rather than
    // quietly switching sending back on.
    let usage = if dirs.config.join(crate::config::FILE).exists() {
        Config::load(dirs)?.usage
    } else {
        Default::default()
    };
    Config {
        vault: vault.root.clone(),
        me: me.clone(),
        usage,
    }
    .save(dirs)?;
    println!(
        "{}",
        terminal(&format!(
            "brain is set up for {me} over {}.\nNext: `brain add <text>`, `brain index`, `brain search <words>`.",
            vault.root.display()
        ))
    );
    // The installed brain starts its own daemon; a build run from anywhere else leaves it to the
    // person, so a dev build never registers a launch agent.
    if service::SUPPORTED && update::runs_from(&update::install_dir(&dirs.bin)) {
        if let Err(e) = start(dirs) {
            eprintln!(
                "{}",
                terminal(&format!(
                    "brain's daemon did not start: {e:#}; run `brain start`"
                ))
            );
        }
    } else {
        println!("To keep the index up to date as notes change, run `brain daemon` in a terminal.");
    }
    Ok(())
}

fn reindex(dirs: &Dirs, clock: &dyn Clock) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let vault = Vault::open(&config.vault)?;
    let Some(_lock) = Lock::try_acquire(&writer_lock(dirs))? else {
        println!(
            "brain's daemon keeps the index up to date, or another brain command is updating it now"
        );
        return Ok(());
    };
    let mut idx = index(dirs)?;
    let r = idx.reindex(&vault)?;
    println!("{}", daemon::summary(&r));
    let provider = match embed::load() {
        Ok(provider) => {
            let e = idx.embed_pending(&*provider, 32)?;
            println!(
                "{} embedded, {} failed, {} pending",
                e.done, e.failed, e.pending
            );
            if let Some(why) = e.stopped {
                println!("{}", terminal(&format!("embedding stopped: {why}")));
            }
            Some(provider)
        }
        Err(e) => {
            println!(
                "{}",
                terminal(&format!(
                    "embeddings skipped, search uses words and links: {e:#}"
                ))
            );
            None
        }
    };
    if r.added + r.changed + r.removed > 0 {
        let filled = gaps::recheck(&idx, &activity(dirs)?, provider.as_deref(), clock)?;
        if filled > 0 {
            println!("{}", plural(filled, "gap") + " filled");
        }
    }
    Ok(())
}

pub fn activity(dirs: &Dirs) -> anyhow::Result<Activity> {
    Activity::open(&dirs.data.join("activity.db"))
}

fn list_gaps(dirs: &Dirs, clock: &dyn Clock, as_json: bool) -> anyhow::Result<()> {
    Config::load(dirs)?;
    let a = activity(dirs)?;
    let open = a.open_gaps(usize::MAX)?;
    let filled = a.filled_since(clock.now() - jiff::SignedDuration::from_hours(30 * 24))?;
    if as_json {
        let rows: Vec<_> = open
            .iter()
            .map(|g| {
                json!({
                    "at": g.at.to_string(),
                    "door": g.door.as_str(),
                    "area": g.area,
                    "query": g.query,
                })
            })
            .collect();
        let v = json!({ "open": rows, "filled_last_30_days": filled });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let mut lines: Vec<String> = open
        .iter()
        .map(|g| {
            format!(
                "{}  {}  {}  {}",
                g.at.strftime("%Y-%m-%d"),
                g.door.as_str(),
                g.area.as_deref().unwrap_or("-"),
                g.query
            )
        })
        .collect();
    if open.is_empty() {
        lines.push("no open gaps".into());
    }
    lines.push(format!("{filled} filled in the last 30 days"));
    println!("{}", terminal(&lines.join("\n")));
    Ok(())
}

fn search(dirs: &Dirs, clock: &dyn Clock, text: &str, s: SearchArgs) -> anyhow::Result<()> {
    Config::load(dirs)?;
    let idx = index(dirs)?;
    let provider = embed::load().ok();
    let q = Query {
        text,
        limit: s.limit,
        kinds: None,
        scope: if s.shared_only {
            Scope::SharedOnly
        } else {
            Scope::Mine
        },
        rerank: embed::runtime().is_ok(),
    };
    let started = std::time::Instant::now();
    let found = idx.search(&q, provider.as_deref())?;
    let ms = started.elapsed().as_millis();
    usage::searched(&usage::store(dirs), clock, Door::Cli, text, &found, ms);
    let gap = is_gap(&found);
    let hits = found.hits;
    if s.json {
        let rows: Vec<_> = hits
            .iter()
            .map(|h| {
                json!({
                    "id": h.note_uuid.map(|u| u.to_string()),
                    "path": h.path,
                    "title": h.title,
                    "kind": h.kind.as_str(),
                    "disclosure": h.disclosure.as_str(),
                    "score": h.score,
                    "snippet": h.snippet,
                })
            })
            .collect();
        let v = json!({
            "gap": gap,
            "hits": rows,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if gap {
        println!("no note answers this; logged as a gap");
    } else if hits.is_empty() {
        println!("nothing found");
    }
    for h in &hits {
        println!(
            "{}",
            terminal(&format!("{}  ({})\n    {}", h.title, h.path, h.snippet))
        );
    }
    Ok(())
}

fn due(dirs: &Dirs, clock: &dyn Clock, within: i64, as_json: bool) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let by = clock.today().checked_add(within.days())?;
    let due = index(dirs)?.due(&config.me, by)?;
    if as_json {
        let rows: Vec<_> = due
            .iter()
            .map(|d| {
                json!({
                    "id": d.note_uuid.map(|u| u.to_string()),
                    "path": d.path,
                    "title": d.title,
                    "review": d.review.to_string(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if due.is_empty() {
        println!("nothing of yours is due");
    }
    for d in &due {
        println!(
            "{}",
            terminal(&format!("{}  {}  {}", d.review, d.path, d.title))
        );
    }
    Ok(())
}

fn show(dirs: &Dirs, target: &str) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let vault = Vault::open(&config.vault)?;
    let path = match uuid::Uuid::parse_str(target) {
        Ok(id) => index(dirs)?
            .path_of(&id)?
            .with_context(|| format!("no indexed note has id {id}; run `brain index`"))?,
        Err(_) => target.to_string(),
    };
    let text = vault
        .read(Path::new(&path))
        .map_err(|why| anyhow::anyhow!("cannot show {path}: {why:?}"))?;
    print!("{}", terminal(&text));
    Ok(())
}

/// `launchctl` as `BRAIN_LAUNCHCTL` names it, so a test can hand brain a stub.
fn launchctl() -> Launchctl {
    Launchctl(
        std::env::var_os("BRAIN_LAUNCHCTL")
            .map_or_else(|| PathBuf::from("/bin/launchctl"), PathBuf::from),
    )
}

/// Loads the daemon as a launchd agent, once everything it needs is in place, so launchd is never
/// handed a daemon that exits at once and restarts every ten seconds.
fn start(dirs: &Dirs) -> anyhow::Result<()> {
    anyhow::ensure!(
        service::SUPPORTED,
        "brain start runs the daemon as a launchd agent, which is macOS only in this release; run `brain daemon` in a terminal instead"
    );
    let exe = update::install_dir(&dirs.bin).join(update::exe_name());
    anyhow::ensure!(
        exe.is_file(),
        "no install at {}; {} first",
        exe.display(),
        service::HOW_TO_INSTALL
    );
    Config::load(dirs)?;
    index(dirs)?;
    let home = std::env::home_dir().context("no home folder for the launch agent")?;
    let log = home.join("Library/Logs/brain/daemon.log");
    let brain_home = std::env::var_os("BRAIN_HOME").map(PathBuf::from);
    launchctl().start(&home, &service::plist(&exe, &log, brain_home.as_deref()))?;
    println!(
        "{}",
        terminal(&format!(
            "brain's daemon is running and starts at every login; its log is {}",
            log.display()
        ))
    );
    Ok(())
}

fn update(dirs: &Dirs, check: bool, source: impl update::Source) -> anyhow::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    if check {
        match update::check(source, version)? {
            Some(newer) => {
                println!("brain {newer} is out; you have {version}. Run `brain update`.")
            }
            None => println!("brain {version} is the newest release."),
        }
        return Ok(());
    }
    let current = update::install_dir(&dirs.bin);
    anyhow::ensure!(
        current.join(update::exe_name()).is_file(),
        "no install at {}; brain updates only an installed brain, so {} first",
        current.display(),
        service::HOW_TO_INSTALL
    );
    let installed = update::run(source, &update::release_trust(), &current, version)?;
    if installed == version {
        println!("brain {version} is the newest release.");
    } else {
        println!("brain {installed} installed over {version}.");
        if launchctl().restart_if_loaded()? {
            println!("restarted the daemon");
        }
    }
    Ok(())
}

fn status(dirs: &Dirs, as_json: bool) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let s = index(dirs)?.stats()?;
    let version = env!("CARGO_PKG_VERSION");
    let runtime = std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from);
    let daemon = Lock::is_held(&writer_lock(dirs));
    if as_json {
        let v = json!({
            "daemon": daemon,
            "runtime": runtime,
            "notes": s.notes,
            "embedded": s.embedded,
            "failed": s.failed,
            "pending": s.pending,
            "skipped": s.skipped,
            "unidentified": s.unidentified,
            "vault": config.vault,
            "version": version,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let mut lines = vec![
        format!("brain {version}, vault {}", config.vault.display()),
        format!(
            "{} notes: {} embedded, {} pending, {} failed, {} without an id",
            s.notes, s.embedded, s.pending, s.failed, s.unidentified
        ),
        match &runtime {
            Some(lib) => format!("ONNX Runtime: {}", lib.display()),
            None => "ONNX Runtime: none, so search uses words and links only".into(),
        },
        format!("daemon: {}", if daemon { "running" } else { "not running" }),
    ];
    for (reason, n) in &s.skipped {
        lines.push(format!("skipped, {reason}: {n}"));
    }
    if s.notes == 0 {
        lines.push("the index is empty; run `brain index`".into());
    }
    println!("{}", terminal(&lines.join("\n")));
    Ok(())
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("{n} {word}")
    } else {
        format!("{n} {word}s")
    }
}

fn import_learning_loop(
    dirs: &Dirs,
    today: jiff::civil::Date,
    a: LearningLoop,
) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let dst = Vault::open(&config.vault)?;
    let src = Vault::open(&a.from)?;
    import::check_apart(&src, &dst)?;
    let share = import::share_folders(&src, &a.share)?;
    let blocklist = a.ll_config.as_deref().map(Blocklist::load).transpose()?;
    let mut ledger = Ledger::load(dirs)?;
    let w = Written {
        me: &config.me,
        today,
        share: &share,
        blocklist: blocklist.as_ref(),
        layout: &dst.layout,
    };
    // Held through the copy too, so a daemon started mid-import waits for it.
    let lock = if a.dry_run {
        None
    } else {
        Lock::try_acquire(&writer_lock(dirs))?
    };
    let (plan, written) = if a.dry_run {
        (
            import::plan(&src, a.memory.as_deref(), &dst, &ledger)?,
            Vec::new(),
        )
    } else {
        // The ledger already holds every copy written before a failure; keeping it lets the next
        // run see those copies as its own instead of as conflicts.
        match import::apply(&src, a.memory.as_deref(), &dst, &mut ledger, &w) {
            Ok(applied) => applied,
            Err(e) => {
                ledger.save(dirs)?;
                return Err(e);
            }
        }
    };
    let mut lines = vec![format!(
        "{} new, {} updated, {} unchanged, {}, {} skipped{}",
        plan.new.len(),
        plan.updated.len(),
        plan.unchanged,
        plural(plan.conflicts.len(), "conflict"),
        plan.skipped.len(),
        if a.dry_run {
            " (dry run: nothing written)"
        } else {
            ""
        }
    )];
    if !plan.conflicts.is_empty() {
        lines.push("changed on both sides, left as they are:".into());
        lines.extend(plan.conflicts.iter().map(|p| format!("  {p}")));
    }
    if !plan.skipped.is_empty() {
        lines.push("skipped:".into());
        lines.extend(
            plan.skipped
                .iter()
                .map(|(k, why)| format!("  {k}: {why:?}")),
        );
    }
    println!("{}\n", terminal(&lines.join("\n")));
    if a.dry_run {
        return Ok(());
    }
    let reindexed = match lock {
        Some(_) => index(dirs).and_then(|mut ix| ix.reindex(&dst)).map(Some),
        None => Ok(None),
    };
    import::settle(&mut ledger, &dst, &written)?;
    ledger.save(dirs)?;
    if reindexed?.is_none() {
        println!(
            "brain's daemon will index them; with no daemon running, run `brain index` once the other brain command finishes\n"
        );
    }
    println!("{}", terminal(&import::report(&dst, today).to_string()));
    Ok(())
}
