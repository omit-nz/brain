use std::path::{Path, PathBuf};

use anyhow::Context;
use brain_index::{Index, Query, Scope, embed};
use brain_note::{Frontmatter, Kind, Split, Vault};
use brain_platform::clock::{Clock, FixedClock, SystemClock};
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Privacy, create_dir_private, write_atomic};
use clap::{Parser, Subcommand, ValueEnum};
use jiff::ToSpan;
use serde_json::json;

use crate::config::{Config, REVIEW_DAYS};
use crate::out::terminal;

#[derive(Parser)]
#[command(
    name = "brain",
    version,
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
    /// Search your notes.
    Search {
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        json: bool,
        /// Reorder the top results with the cross-encoder.
        #[arg(long)]
        rerank: bool,
        /// Only what a colleague could see.
        #[arg(long)]
        shared_only: bool,
    },
    /// Write a new note into the inbox.
    Add {
        text: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
    },
    /// Print a note, by id or by its path in the vault.
    Show { target: String },
    /// What the index holds.
    Status {
        #[arg(long)]
        json: bool,
    },
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

/// Lowercase words of `text` joined by `-`, at most 60 characters.
fn slug(text: &str) -> String {
    let mut s = String::new();
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if s.len() + word.len() > 60 {
            break;
        }
        if !s.is_empty() {
            s.push('-');
        }
        s.push_str(&word.to_lowercase());
    }
    if s.is_empty() { "note".into() } else { s }
}

pub fn run(cli: Cli, dirs: &Dirs) -> anyhow::Result<()> {
    let clock: Box<dyn Clock> = match cli.now {
        Some(t) => Box::new(FixedClock(t)),
        None => Box::new(SystemClock),
    };
    match cli.command {
        Command::Init { me, vault } => init(dirs, me, vault),
        Command::Index => reindex(dirs),
        Command::Search {
            text,
            limit,
            json,
            rerank,
            shared_only,
        } => search(dirs, &text.join(" "), limit, json, rerank, shared_only),
        Command::Add { text, title, kind } => {
            add(dirs, &*clock, &text, title, kind.map(Kind::from))
        }
        Command::Show { target } => show(dirs, &target),
        Command::Status { json } => status(dirs, json),
    }
}

fn init(dirs: &Dirs, me: String, vault: Option<PathBuf>) -> anyhow::Result<()> {
    let root = match vault {
        Some(v) => v,
        None => std::env::current_dir()?,
    };
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
    Config {
        vault: vault.root.clone(),
        me: me.clone(),
    }
    .save(dirs)?;
    println!(
        "{}",
        terminal(&format!(
            "brain is set up for {me} over {}.\nNext: `brain add <text>`, `brain index`, `brain search <words>`.",
            vault.root.display()
        ))
    );
    Ok(())
}

fn reindex(dirs: &Dirs) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let vault = Vault::open(&config.vault)?;
    let mut idx = index(dirs)?;
    let r = idx.reindex(&vault)?;
    println!(
        "{} added, {} changed, {} removed, {} skipped, {} ids written, {} without an id",
        r.added,
        r.changed,
        r.removed,
        r.skipped.len(),
        r.ids_written,
        r.unidentified.len()
    );
    match embed::load() {
        Ok(provider) => {
            let e = idx.embed_pending(&*provider, 32)?;
            println!(
                "{} embedded, {} failed, {} pending",
                e.done, e.failed, e.pending
            );
            if let Some(why) = e.stopped {
                println!("{}", terminal(&format!("embedding stopped: {why}")));
            }
        }
        Err(e) => println!(
            "{}",
            terminal(&format!(
                "embeddings skipped, search uses words and links: {e:#}"
            ))
        ),
    }
    Ok(())
}

fn search(
    dirs: &Dirs,
    text: &str,
    limit: usize,
    as_json: bool,
    rerank: bool,
    shared_only: bool,
) -> anyhow::Result<()> {
    Config::load(dirs)?;
    let idx = index(dirs)?;
    let provider = embed::load().ok();
    let q = Query {
        text,
        limit,
        kinds: None,
        scope: if shared_only {
            Scope::SharedOnly
        } else {
            Scope::Mine
        },
        rerank,
    };
    let hits = idx.search(&q, provider.as_deref())?;
    if as_json {
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
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if hits.is_empty() {
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

fn add(
    dirs: &Dirs,
    clock: &dyn Clock,
    text: &str,
    title: Option<String>,
    kind: Option<Kind>,
) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let vault = Vault::open(&config.vault)?;
    let today = clock.today();
    let review = today.checked_add(REVIEW_DAYS.days())?;
    let mut fm = Frontmatter::new_with("id", &uuid::Uuid::now_v7().to_string(), "\n")
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut fields = Vec::new();
    if let Some(t) = &title {
        fields.push(("title", t.clone()));
    }
    fields.push(("date", today.to_string()));
    fields.push(("owner", config.me.clone()));
    fields.push(("review", review.to_string()));
    if let Some(k) = kind {
        fields.push(("kind", k.as_str().to_string()));
    }
    for (key, value) in fields {
        fm.set(key, &value)
            .map_err(|_| anyhow::anyhow!("{key} cannot hold a line break"))?;
    }
    let body = format!("{}\n", text.trim_end());
    let note = Split {
        bom: false,
        fm: Ok(Some(fm)),
        body: &body,
    }
    .render();
    let base = slug(title.as_deref().unwrap_or(text));
    let inbox = &vault.layout.inbox;
    let mut rel = inbox.join(format!("{base}.md"));
    let mut n = 0;
    while vault.root.join(&rel).exists() {
        n += 1;
        rel = inbox.join(format!("{base}-{n}.md"));
    }
    write_atomic(&vault.root, &rel, note.as_bytes(), Privacy::Normal)?;
    println!("{}", terminal(&brain_note::rel_key(&rel)));
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

fn status(dirs: &Dirs, as_json: bool) -> anyhow::Result<()> {
    let config = Config::load(dirs)?;
    let s = index(dirs)?.stats()?;
    let version = env!("CARGO_PKG_VERSION");
    if as_json {
        let v = json!({
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
