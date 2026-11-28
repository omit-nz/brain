//! `brain setup`: what it takes, what it asks, and the order it writes in. The config is written
//! last, so whoever finds it finds the folder and the index in place.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use brain_index::Index;
use brain_note::Vault;
use brain_platform::account;
use brain_platform::clock::Clock;
use brain_platform::dirs::Dirs;
use brain_platform::fs::{Lock, create_dir_private};

use crate::cli::{self, Switch};
use crate::config::Config;
use crate::import::Ledger;
use crate::offer;
use crate::out::terminal;
use crate::usage;

#[derive(clap::Args)]
pub struct SetupArgs {
    /// Take the defaults and ask nothing (what a bare `brain setup` does).
    #[arg(long, conflicts_with = "guided")]
    auto: bool,
    /// Be asked each choice.
    #[arg(long)]
    guided: bool,
    /// Your name, as brain writes it on your notes.
    #[arg(long)]
    me: Option<String>,
    /// The folder for your notes, as a full path.
    #[arg(long)]
    vault: Option<PathBuf>,
    /// Keep copies of Claude Code's auto-memory.
    #[arg(long, value_enum)]
    memory: Option<Switch>,
    /// Send the day's usage counts to omit.
    #[arg(long, value_enum)]
    usage: Option<Switch>,
}

pub fn run(dirs: &Dirs, clock: &dyn Clock, args: SetupArgs) -> anyhow::Result<()> {
    let guided = args.guided && !args.auto;
    // An unreadable config stops here, before anything is asked or written.
    let before = Config::find(dirs)?;
    let home = std::env::home_dir().context("this computer names no home folder")?;
    let projects = home.join(".claude").join("projects");

    let held = before.as_ref().map(|c| c.me.clone());
    let mut found = None;
    let given = match args.me {
        Some(me) => Some(me),
        None if guided => {
            let kept = held.clone().or_else(|| {
                found = account::name();
                found.clone()
            });
            ask("Your name", kept.as_deref().unwrap_or(""))?
        }
        None => None,
    };
    let me = name(given, held, || found.or_else(account::name))?;

    let configured = before.as_ref().map(|c| c.vault.clone());
    let given = match args.vault {
        Some(vault) => Some(vault),
        None if guided => {
            let kept = configured.clone().or_else(|| default_folder(&home).ok());
            let kept = kept.map(|p| p.display().to_string()).unwrap_or_default();
            ask("Folder for your notes", &kept)?.map(PathBuf::from)
        }
        None => None,
    };
    let root = notes_folder(dirs, &home, given, configured)?;

    let followed = before.as_ref().is_some_and(|c| c.memory.follow.is_some());
    let memory = match args.memory {
        None if guided => ask_switch("Keep copies of Claude Code's auto-memory?", followed)?,
        memory => memory,
    };
    let sending = before.as_ref().is_none_or(|c| c.usage.send);
    let send = match args.usage {
        None if guided => ask_switch("Send the day's usage counts to omit?", sending)?,
        send => send,
    };

    create_dir_private(&dirs.data)?;
    let vault = make_vault(&root, true)?;
    Index::open(&dirs.data.join("index.db"))?;
    {
        let held = Lock::acquire(&dirs.data.join(offer::LOCK))?;
        if let Err(e) = offer::tidy(&dirs.data, clock, &held) {
            eprintln!("{}", terminal(&format!("brain: {e:#}")));
        }
    }
    let found = followable(&projects);
    let mut sent = true;
    let config = Config::update(dirs, |before| {
        let mut config = before.unwrap_or_else(|| Config::new(vault.root.clone(), me.clone()));
        sent = config.usage.send;
        config.vault = vault.root.clone();
        config.me = me.clone();
        match memory {
            Some(Switch::On) => config.memory.follow = config.memory.follow.take().or(found),
            Some(Switch::Off) => config.memory.follow = None,
            None => {}
        }
        if let Some(send) = send {
            config.usage.send = matches!(send, Switch::On);
        }
        config.setup.ran = Some(clock.today());
        Ok(config)
    })?;
    if config.usage.send != sent {
        usage::mark_switched(dirs, clock.today())?;
    }

    let memory_line = match (&config.memory.follow, memory) {
        (Some(folder), _) => format!(
            "Auto-memory: followed from {}; its copies are private notes.",
            folder.display()
        ),
        (None, Some(Switch::On)) => format!(
            "Auto-memory: no folder at {} yet; `brain memory follow <folder>` follows one later.",
            projects.display()
        ),
        (None, _) => "Auto-memory: not followed; `brain setup --memory on` follows it.".into(),
    };
    let usage_line = if config.usage.send {
        "Usage counts to omit: on; `brain usage off` stops them."
    } else {
        "Usage counts to omit: off."
    };
    println!(
        "{}",
        terminal(&format!(
            "brain is set up for {me} over {}.\n{memory_line}\n{usage_line}",
            vault.root.display()
        ))
    );
    cli::start_daemon(dirs);
    println!("To connect Claude Code, run `brain connect claude-code`, then start a new session.");
    Ok(())
}

/// The name to set up under: the one given, else the one set up before, else the computer's.
fn name(
    given: Option<String>,
    held: Option<String>,
    found: impl FnOnce() -> Option<String>,
) -> anyhow::Result<String> {
    given
        .or(held)
        .or_else(found)
        .context("brain setup needs your name: run brain setup --me <name>")
}

/// A question and what an empty answer keeps; `None` for that, or for no more input.
fn ask(question: &str, kept: &str) -> anyhow::Result<Option<String>> {
    print!("{}", terminal(&format!("{question} [{kept}]: ")));
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let line = line.trim();
    Ok((!line.is_empty()).then(|| line.to_string()))
}

fn ask_switch(question: &str, kept: bool) -> anyhow::Result<Option<Switch>> {
    loop {
        let kept = if kept { "on" } else { "off" };
        match ask(&format!("{question} (on/off)"), kept)?.as_deref() {
            None => return Ok(None),
            Some("on") => return Ok(Some(Switch::On)),
            Some("off") => return Ok(Some(Switch::Off)),
            Some(_) => {}
        }
    }
}

fn same(a: &Path, b: &Path) -> bool {
    a.canonicalize()
        .ok()
        .is_some_and(|a| Some(a) == b.canonicalize().ok())
}

/// The folder setup works over: the one given, else the one set up before, else brain's own.
fn notes_folder(
    dirs: &Dirs,
    home: &Path,
    given: Option<PathBuf>,
    configured: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    let Some(given) = given else {
        let Some(old) = configured else {
            return default_folder(home);
        };
        // Never made again: it may be on a disk that is not connected.
        ensure!(
            old.is_dir(),
            "the vault folder {} is not there; connect it or name another with --vault",
            old.display()
        );
        return Ok(old);
    };
    // A typed answer meets no shell.
    let root = match given.strip_prefix("~") {
        Ok(under) => home.join(under),
        Err(_) => given,
    };
    ensure!(root.is_absolute(), "give the notes folder as a full path");
    if let Some(old) = configured
        && !same(&root, &old)
        && Ledger::load(dirs)?.holds_memory(&old)
    {
        bail!(
            "brain keeps copies of your auto-memory in {}, and they stop following if the folder \
             changes; keep it, or run brain init if you mean to switch",
            old.display()
        );
    }
    Ok(root)
}

/// `~/brain`, or `~/brain-notes`, whichever is first free to be brain's.
fn default_folder(home: &Path) -> anyhow::Result<PathBuf> {
    let free = |root: &Path| match std::fs::read_dir(root) {
        Ok(mut entries) => {
            entries.next().is_none()
                || ["0-inbox", "working-memory"]
                    .iter()
                    .all(|folder| root.join(folder).is_dir())
        }
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    };
    ["brain", "brain-notes"]
        .iter()
        .map(|name| home.join(name))
        .find(|root| free(root))
        .context(
            "the folders brain and brain-notes in your home folder hold other things; name a \
             notes folder with --vault",
        )
}

/// The memory folder to follow, when it is one: a link is kept as what it points at, since the
/// pass follows no link.
pub fn followable(projects: &Path) -> Option<PathBuf> {
    projects.canonicalize().ok().filter(|p| p.is_dir())
}

/// `root` as a vault, with the folders brain keeps in one. brain writes an id into every note
/// under its vault, so a folder holding far more than notes is refused; a git repository only
/// when the folder was not `named`, since a notes folder synced with git is a vault once its
/// owner names it.
pub fn make_vault(root: &Path, named: bool) -> anyhow::Result<Vault> {
    let what = if std::env::home_dir().is_some_and(|home| same(root, &home)) {
        Some("your home folder")
    } else if !named && root.join(".git").exists() {
        Some("a code repository")
    } else if root.canonicalize().is_ok_and(|r| r.parent().is_none()) {
        Some("the root of the disk")
    } else {
        None
    };
    if let Some(what) = what {
        bail!(
            "{} is {what}; brain writes an id into every note in its vault, so name your notes \
             folder with --vault",
            root.display()
        );
    }
    if !root.exists() {
        create_dir_private(root)?;
    }
    let vault = Vault::open(root)?;
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
    Ok(vault)
}

#[cfg(test)]
mod tests {
    use super::name;

    #[test]
    fn no_name_anywhere_asks_for_one() {
        let held = name(None, Some("Test Person".into()), || None);
        assert_eq!(held.unwrap(), "Test Person");
        let refused = name(None, None, || None).unwrap_err().to_string();
        assert!(refused.contains("--me"), "{refused}");
    }
}
