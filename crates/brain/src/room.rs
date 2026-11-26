//! Rooms: a room is the vault folder `rooms/<name>/` and a post is a private note in it, named by
//! its UTC second and title so the folder lists in posting order. Each reader keeps the set of
//! posts it has been shown in brain's data folder, never in the vault, so it never syncs.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use brain_note::{Frontmatter, MAX_NOTE_BYTES, Split, Vault, rel_key, text_of};
use brain_platform::fs::{Lock, Privacy, create_dir_private, create_new, is_link, write_atomic};
use jiff::Timestamp;
use sha2::{Digest, Sha256};

use crate::capture::slug;

pub const DEFAULT_ROOM: &str = "squads";
const ROOMS: &str = "rooms";
const TITLE_CHARS: usize = 80;
const MAX_TITLE_CHARS: usize = 200;
/// Posts shown by one `new`.
pub const PAGE: usize = 50;

/// A room name: `^[a-z0-9][a-z0-9-]{0,39}$`. Anything else is refused, never rewritten, so a
/// name can never reach outside `rooms/`.
pub fn check_room(name: &str) -> anyhow::Result<()> {
    let ok = name.len() <= 40
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    anyhow::ensure!(
        ok,
        "{name:?} is not a room name: 1 to 40 lowercase letters, digits and dashes, starting with a letter or digit"
    );
    Ok(())
}

/// A reader's or author's name: 1 to 60 characters on one line.
pub fn check_name(name: &str) -> anyhow::Result<()> {
    let n = name.chars().count();
    anyhow::ensure!(
        (1..=60).contains(&n) && !name.chars().any(char::is_control),
        "a name is 1 to 60 characters on one line"
    );
    Ok(())
}

/// A post just written: its vault path, id and title.
pub struct Posted {
    pub rel: String,
    pub id: uuid::Uuid,
    pub title: String,
}

/// The text's first non-empty line without its control characters, trimmed and cut at 80
/// characters.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(|l| l.replace(char::is_control, "").trim().to_string())
        .find(|l| !l.is_empty());
    line.unwrap_or_default().chars().take(TITLE_CHARS).collect()
}

/// Writes `text` as a post by `author` into `room`, named
/// `<UTC time as YYYYMMDDTHHMMSSZ>-<slug of the title>.md`, then `-1`, `-2` and on, taking the
/// first name no file holds, so two writers in one second never overwrite each other.
pub fn post(
    vault: &Vault,
    room: &str,
    author: &str,
    now: Timestamp,
    text: &str,
    title: Option<&str>,
) -> anyhow::Result<Posted> {
    check_room(room)?;
    check_name(author)?;
    anyhow::ensure!(!text.trim().is_empty(), "a post is empty");
    let title = match title {
        Some(t) => {
            anyhow::ensure!(
                t.chars().count() <= MAX_TITLE_CHARS && !t.chars().any(char::is_control),
                "a title is at most 200 characters, with no line break or other control character"
            );
            t.to_string()
        }
        None => first_line(text),
    };
    let id = uuid::Uuid::now_v7();
    let mut fm =
        Frontmatter::new_with("id", &id.to_string(), "\n").map_err(|e| anyhow::anyhow!("{e:?}"))?;
    for (key, value) in [
        ("room", room.to_string()),
        ("author", author.to_string()),
        ("posted", now.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()),
        ("title", title.clone()),
        // Until rooms have permissions, a post never reaches colleague search or a sync.
        ("visibility", "private".to_string()),
    ] {
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
    anyhow::ensure!(
        note.len() as u64 <= MAX_NOTE_BYTES,
        "a post is at most 2 MiB"
    );
    let base = format!("{}-{}", now.strftime("%Y%m%dT%H%M%SZ"), slug(&title));
    let dir = folder(room);
    for n in 0.. {
        let name = match n {
            0 => format!("{base}.md"),
            n => format!("{base}-{n}.md"),
        };
        let rel = dir.join(name);
        match create_new(&vault.root, &rel, note.as_bytes(), Privacy::Normal) {
            Ok(()) => {
                return Ok(Posted {
                    rel: rel_key(&rel),
                    id,
                    title,
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("writing {}", rel_key(&rel))),
        }
    }
    unreachable!("a name is always free")
}

fn folder(room: &str) -> PathBuf {
    Path::new(ROOMS).join(room)
}

fn refuse_link(vault: &Vault, rel: &Path) -> anyhow::Result<()> {
    let at = vault.root.join(rel);
    anyhow::ensure!(
        !is_link(&at),
        "{} is a link; brain reads rooms only where they live",
        rel_key(rel)
    );
    Ok(())
}

/// The post file names in `room`, in posting order: plain `.md` files not starting with `.`, so
/// a writer's temporary file is never read half written. A missing room holds none.
fn post_names(vault: &Vault, room: &str) -> anyhow::Result<Vec<String>> {
    refuse_link(vault, Path::new(ROOMS))?;
    let dir = folder(room);
    refuse_link(vault, &dir)?;
    let entries = match fs::read_dir(vault.root.join(&dir)) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", rel_key(&dir))),
    };
    let mut names = Vec::new();
    for e in entries {
        let e = e?;
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if e.file_type()?.is_file() && name.ends_with(".md") && !name.starts_with('.') {
            names.push(name);
        }
    }
    // By name without `.md`, so `<base>.md` comes before the `<base>-1.md` written after it.
    names.sort_by(|a, b| stem(a).cmp(stem(b)));
    Ok(names)
}

fn stem(name: &str) -> &str {
    name.strip_suffix(".md").unwrap_or(name)
}

pub struct Message {
    pub id: String,
    pub room: String,
    pub author: String,
    pub posted: String,
    pub title: String,
    pub body: String,
}

pub struct Post {
    pub file: String,
    pub rel: String,
    /// The post, or why it could not be read.
    pub read: Result<Message, String>,
}

/// One `new`: the oldest posts the reader has not been shown, and how many more wait.
pub struct Page {
    pub posts: Vec<Post>,
    pub more: usize,
}

/// A post as its frontmatter says; one written without frontmatter (by hand, in a notes app) is
/// its text, its time read from its file name and its title its first line.
fn read_post(vault: &Vault, room: &str, file: &str) -> Result<Message, String> {
    let text = vault
        .read(&folder(room).join(file))
        .map_err(|why| why.describe())?;
    let split = Split::parse(&text);
    let get = |k: &str| match &split.fm {
        Ok(Some(fm)) => text_of(fm.get(k)).unwrap_or_default(),
        _ => String::new(),
    };
    if split.fm.is_err() {
        return Err("its frontmatter does not parse".into());
    }
    let title = get("title");
    Ok(Message {
        id: get("id"),
        room: room.to_string(),
        author: get("author"),
        posted: match get("posted") {
            t if t.is_empty() => time_of(file).unwrap_or_default(),
            t => t,
        },
        title: if title.is_empty() {
            first_line(split.body)
        } else {
            title
        },
        body: split.body.to_string(),
    })
}

/// Where `reader`'s seen set for `room` lives, under brain's data folder: named by a hash, so
/// any name is a safe file name and two pairs never share a file.
fn seen_file(room: &str, reader: &str) -> PathBuf {
    let hash = Sha256::digest(format!("{room}\0{reader}").as_bytes());
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    Path::new(ROOMS).join(hex)
}

/// The posts in `room` that `reader` has not been shown, oldest first, at most `PAGE`, handed
/// to `render`, which returns its reply and how many of the page's first posts the reply holds.
/// Only those are added to the reader's seen set, and only once the reply is built, so neither a
/// failure nor a reply cut short ever skips a post. A set rather than a last-seen mark, so a post
/// written late in a second already read is still new. One reader at a time: two sessions under
/// one name would otherwise both be shown the same page.
pub fn read_new<T>(
    vault: &Vault,
    data: &Path,
    room: &str,
    reader: &str,
    render: impl FnOnce(&Page) -> (T, usize),
) -> anyhow::Result<T> {
    check_room(room)?;
    check_name(reader)?;
    create_dir_private(data)?;
    let _one_reader = Lock::acquire(&data.join("rooms.lock"))?;
    let rel = seen_file(room, reader);
    let at = data.join(&rel);
    // Read as far as it goes: a damaged line only means its post is shown again.
    let seen: BTreeSet<String> = match fs::read(&at) {
        Ok(b) => String::from_utf8_lossy(&b)
            .lines()
            .map(String::from)
            .collect(),
        Err(e) if e.kind() == ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => {
            return Err(e)
                .with_context(|| format!("reading what this reader has seen, {}", at.display()));
        }
    };
    let names = post_names(vault, room)?;
    let unseen: Vec<&String> = names.iter().filter(|n| !seen.contains(*n)).collect();
    let more = unseen.len().saturating_sub(PAGE);
    let posts: Vec<Post> = unseen
        .into_iter()
        .take(PAGE)
        .map(|file| Post {
            read: read_post(vault, room, file),
            rel: rel_key(&folder(room).join(file)),
            file: file.clone(),
        })
        .collect();
    let page = Page { posts, more };
    let (out, shown) = render(&page);
    if shown > 0 {
        let shown: BTreeSet<&String> = page.posts[..shown].iter().map(|p| &p.file).collect();
        // Only names still in the room, so the set never outgrows it.
        let text: String = names
            .iter()
            .filter(|n| seen.contains(*n) || shown.contains(n))
            .map(|n| format!("{n}\n"))
            .collect();
        write_atomic(data, &rel, text.as_bytes(), Privacy::Private)
            .with_context(|| format!("saving what this reader has seen, {}", at.display()))?;
    }
    Ok(out)
}

pub struct RoomInfo {
    pub name: String,
    /// How many posts it holds and the newest one's time (RFC 3339, read from its file name),
    /// or why its folder cannot be read.
    pub posts: Result<(usize, Option<String>), String>,
}

/// `20261126T080000Z-...` as `2026-11-26T08:00:00Z`.
fn time_of(file: &str) -> Option<String> {
    let t = file.get(..16)?;
    let ts = jiff::fmt::strtime::parse("%Y%m%dT%H%M%SZ", t)
        .ok()?
        .to_datetime()
        .ok()?
        .to_zoned(jiff::tz::TimeZone::UTC)
        .ok()?
        .timestamp();
    Some(ts.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// Every room, by name; a folder whose name is not a room name is not a room.
pub fn list(vault: &Vault) -> anyhow::Result<Vec<RoomInfo>> {
    refuse_link(vault, Path::new(ROOMS))?;
    let entries = match fs::read_dir(vault.root.join(ROOMS)) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).context("reading rooms"),
    };
    let mut rooms = Vec::new();
    for e in entries {
        let e = e?;
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if !e.file_type()?.is_dir() || check_room(&name).is_err() {
            continue;
        }
        let posts = post_names(vault, &name)
            .map(|names| (names.len(), names.last().and_then(|n| time_of(n))))
            .map_err(|e| format!("{e:#}"));
        rooms.push(RoomInfo { name, posts });
    }
    rooms.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rooms)
}
