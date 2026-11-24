use std::io::{self, ErrorKind};
use std::path::{Component, Path};

use brain_platform::fs::{Privacy, write_atomic};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::disclosure::{Disclosure, Place, disclosure};
use crate::frontmatter::{FmError, Frontmatter, Split, Value, text_of};
use crate::links::wikilinks;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Knowledge,
    /// Working memory: how we work here.
    Rule,
    History,
}

impl Kind {
    /// The `kind` value as written in frontmatter and stored in the index; anything else reads as
    /// knowledge.
    pub fn parse(v: &str) -> Kind {
        match v {
            "rule" => Kind::Rule,
            "history" => Kind::History,
            _ => Kind::Knowledge,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Knowledge => "knowledge",
            Kind::Rule => "rule",
            Kind::History => "history",
        }
    }

    fn from_value(v: Option<String>) -> Kind {
        v.as_deref().map_or(Kind::Knowledge, Kind::parse)
    }
}

#[derive(Debug, Clone)]
pub struct Note {
    pub id: Option<Uuid>,
    pub key: String,
    pub title: String,
    pub tags: Vec<String>,
    pub kind: Kind,
    pub owner: Option<String>,
    pub review: Option<jiff::civil::Date>,
    pub disclosure: Disclosure,
    pub body: String,
    pub links: Vec<String>,
    pub hash: [u8; 32],
}

/// A vault path as stored and printed everywhere: components joined with `/` on every platform.
pub fn rel_key(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

impl Note {
    /// Never fails: broken frontmatter gives a note with no metadata, the whole text as its body,
    /// and nothing disclosed.
    pub fn parse(rel: &Path, raw: &str, place: Place) -> Note {
        let split = Split::parse(raw);
        let fm = split.fm.as_ref().ok().and_then(|f| f.as_ref());
        let get = |k: &str| fm.and_then(|f| text_of(f.get(k)));
        let tags = match fm.and_then(|f| f.get("tags")) {
            Some(Value::List(v)) => v,
            Some(Value::Text(t)) if !t.is_empty() => vec![t],
            _ => Vec::new(),
        };
        let heading = split
            .body
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .map(|h| h.trim().to_string());
        let stem = rel
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        Note {
            id: get("id").and_then(|s| Uuid::parse_str(&s).ok()),
            key: rel_key(rel),
            title: get("title").or(heading).unwrap_or(stem),
            tags,
            kind: Kind::from_value(get("kind")),
            owner: get("owner"),
            review: get("review").and_then(|s| s.parse().ok()),
            disclosure: disclosure(&split, place),
            body: split.body.to_string(),
            links: wikilinks(split.body),
            hash: Sha256::digest(raw.as_bytes()).into(),
        }
    }
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, msg.to_string())
}

fn write_id(root: &Path, rel: &Path, mut split: Split<'_>, id: Uuid) -> io::Result<()> {
    match &mut split.fm {
        Err(_) => return Err(invalid("broken frontmatter: no id is written into it")),
        Ok(Some(fm)) => fm
            .set("id", &id.to_string())
            .map_err(|_: FmError| invalid("id written twice: no id is written into it"))?,
        Ok(None) => {
            let crlf = split
                .body
                .find('\n')
                .is_some_and(|i| split.body[..i].ends_with('\r'));
            let eol = if crlf { "\r\n" } else { "\n" };
            split.fm = Ok(Some(
                Frontmatter::new_with("id", &id.to_string(), eol)
                    .map_err(|_| invalid("unsafe id"))?,
            ));
        }
    }
    write_atomic(root, rel, split.render().as_bytes(), Privacy::Normal)
}

/// The note's id, writing a new UUIDv7 only when frontmatter is valid and holds none.
pub fn ensure_id(root: &Path, rel: &Path, raw: &str) -> io::Result<(Uuid, bool)> {
    let split = Split::parse(raw);
    match &split.fm {
        Err(_) => return Err(invalid("broken frontmatter: no id is written into it")),
        Ok(Some(fm)) => {
            if let Some(text) = text_of(fm.get("id")) {
                let id = Uuid::parse_str(&text).map_err(|_| invalid("id is not a UUID"))?;
                return Ok((id, false));
            }
        }
        Ok(None) => {}
    }
    let id = Uuid::now_v7();
    write_id(root, rel, split, id)?;
    Ok((id, true))
}

/// A fresh id for a note whose id another note already holds.
pub fn reassign_id(root: &Path, rel: &Path, raw: &str) -> io::Result<Uuid> {
    let id = Uuid::now_v7();
    write_id(root, rel, Split::parse(raw), id)?;
    Ok(id)
}
