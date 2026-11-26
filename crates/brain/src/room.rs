//! Rooms: a room is the vault folder `rooms/<name>/` and a post is a private note in it, named by
//! its UTC second and title so the folder lists in posting order. Each reader keeps the set of
//! posts it has been shown in brain's data folder, never in the vault, so it never syncs.

use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use brain_note::{Frontmatter, MAX_NOTE_BYTES, Split, Vault, rel_key, text_of};
use brain_platform::fs::{Privacy, create_new, is_link, write_atomic};
use jiff::Timestamp;
use sha2::{Digest, Sha256};

use crate::capture::slug;

pub const DEFAULT_ROOM: &str = "squads";
const ROOMS: &str = "rooms";
const TITLE_CHARS: usize = 80;
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

/// The text's first non-empty line, trimmed and cut at 80 characters.
fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
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
        Some(t) => t.to_string(),
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
    names.sort();
    Ok(names)
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

fn read_post(vault: &Vault, rel: &Path) -> Result<Message, String> {
    let text = vault.read(rel).map_err(|why| format!("{why:?}"))?;
    let split = Split::parse(&text);
    let fm = match &split.fm {
        Ok(Some(fm)) => fm,
        Ok(None) => return Err("no frontmatter".into()),
        Err(e) => return Err(format!("frontmatter {e:?}")),
    };
    let get = |k: &str| text_of(fm.get(k)).unwrap_or_default();
    Ok(Message {
        id: get("id"),
        room: get("room"),
        author: get("author"),
        posted: get("posted"),
        title: get("title"),
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
/// to `render`; only once the reply is built are they added to the reader's seen set, so a
/// failure never skips a post. A set rather than a last-seen mark, so a post written late in a
/// second already read is still new.
pub fn read_new<T>(
    vault: &Vault,
    data: &Path,
    room: &str,
    reader: &str,
    render: impl FnOnce(&Page) -> T,
) -> anyhow::Result<T> {
    check_room(room)?;
    check_name(reader)?;
    let rel = seen_file(room, reader);
    let mut seen: BTreeSet<String> = match fs::read_to_string(data.join(&rel)) {
        Ok(t) => t.lines().map(String::from).collect(),
        Err(e) if e.kind() == ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => return Err(e).context("reading what this reader has seen"),
    };
    let unseen: Vec<String> = post_names(vault, room)?
        .into_iter()
        .filter(|n| !seen.contains(n))
        .collect();
    let more = unseen.len().saturating_sub(PAGE);
    let posts: Vec<Post> = unseen
        .into_iter()
        .take(PAGE)
        .map(|file| {
            let rel = folder(room).join(&file);
            Post {
                read: read_post(vault, &rel),
                rel: rel_key(&rel),
                file,
            }
        })
        .collect();
    let page = Page { posts, more };
    let out = render(&page);
    if !page.posts.is_empty() {
        seen.extend(page.posts.into_iter().map(|p| p.file));
        let text: String = seen.into_iter().map(|n| n + "\n").collect();
        write_atomic(data, &rel, text.as_bytes(), Privacy::Private)
            .context("saving what this reader has seen")?;
    }
    Ok(out)
}

pub struct RoomInfo {
    pub name: String,
    pub posts: usize,
    /// The newest post's time, RFC 3339, read from its file name.
    pub newest: Option<String>,
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
        let names = post_names(vault, &name)?;
        rooms.push(RoomInfo {
            posts: names.len(),
            newest: names.last().and_then(|n| time_of(n)),
            name,
        });
    }
    rooms.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rooms)
}
