//! The one function that decides what a note may disclose. Every doubt withholds: with shared as
//! the default, a path that failed open would publish a note against its author's intent.

use std::sync::LazyLock;

use regex::Regex;

use crate::frontmatter::{Split, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Not yet kept: a note is shared only once its person keeps it.
    Inbox,
    /// The person's private folder.
    Private,
    Kept,
}

impl Place {
    pub fn as_str(self) -> &'static str {
        match self {
            Place::Inbox => "inbox",
            Place::Private => "private",
            Place::Kept => "kept",
        }
    }

    /// The place `as_str` wrote; anything else reads as private, the place that shares nothing.
    pub fn parse(v: &str) -> Place {
        match v {
            "inbox" => Place::Inbox,
            "kept" => Place::Kept,
            _ => Place::Private,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    Shared,
    /// Findable but not readable: a title and a scrubbed summary.
    Listed,
    Private,
}

impl Disclosure {
    /// The stored spelling; anything unknown reads as private.
    pub fn parse(v: &str) -> Disclosure {
        match v {
            "shared" => Disclosure::Shared,
            "listed" => Disclosure::Listed,
            _ => Disclosure::Private,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Disclosure::Shared => "shared",
            Disclosure::Listed => "listed",
            Disclosure::Private => "private",
        }
    }
}

pub fn disclosure(split: &Split<'_>, place: Place) -> Disclosure {
    if place != Place::Kept {
        return Disclosure::Private;
    }
    let fm = match &split.fm {
        Err(_) => return Disclosure::Private,
        Ok(None) => return Disclosure::Shared,
        Ok(Some(fm)) => fm,
    };
    if fm.misplaced("visibility") {
        return Disclosure::Private;
    }
    match fm.get("visibility") {
        None => Disclosure::Shared,
        Some(Value::Text(v)) => match normalise(&v).as_str() {
            "shared" | "public" => Disclosure::Shared,
            "listed" => Disclosure::Listed,
            _ => Disclosure::Private,
        },
        Some(_) => Disclosure::Private,
    }
}

/// learning-loop's normalising: an unquoted value loses a trailing `# comment`, and only
/// matched quotes are stripped, so a half-quoted value stays unknown. As in YAML, a `#` starts a
/// comment only after a space or tab: `shared#x` is one unknown value, not `shared`.
fn normalise(raw: &str) -> String {
    let mut v = raw.trim();
    if !v.starts_with(['"', '\'']) {
        let comment = v
            .char_indices()
            .find(|&(i, c)| c == '#' && v[..i].ends_with([' ', '\t']))
            .map_or(v.len(), |(i, _)| i);
        v = v[..comment].trim();
    }
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            v = &v[1..v.len() - 1];
            break;
        }
    }
    v.trim().to_ascii_lowercase()
}

const LISTED_CHARS: usize = 300;

/// learning-loop's credential patterns, in its order.
static SECRETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"AKIA[0-9A-Z]{16}",
        r"gh[po]_[A-Za-z0-9]{36,}",
        r"sk-ant-api[A-Za-z0-9_-]{20,}",
        r"sk_(?:live|test)_[A-Za-z0-9]{20,}",
        r"sk-[A-Za-z0-9_-]{20,}",
        r"cfpat-[A-Za-z0-9_-]{20,}",
        r"Bearer\s+[A-Za-z0-9._\-/+=]{20,}",
        r"xox[abprs]-[A-Za-z0-9-]{10,}",
        r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
        // A key with no END line runs to the end of the text.
        r"(?s:-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z))",
        r"(?i)([a-z][a-z0-9+.-]*://)[^\s/:@]+:[^\s/@]+@",
        r#"(?i)\b(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|client[_-]?secret)\b(\s*[=:]\s*)("[^\s"']{6,}"|'[^\s"']{6,}'|[^\s"']{6,})"#,
        r"(?i)\bAuthorization:\s*Basic\s+[A-Za-z0-9+/=]{8,}",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("secret pattern compiles"))
    .collect()
});

/// The first paragraph of the scrubbed body, then cut: the whole body is scrubbed first, so a
/// secret that a paragraph break or the cut would split is still caught by its full pattern.
pub fn listed_summary(body: &str) -> String {
    let mut text = body.replace("\r\n", "\n");
    for re in SECRETS.iter() {
        text = re.replace_all(&text, "[REDACTED]").into_owned();
    }
    let s = text.trim_start().split("\n\n").next().unwrap_or("").trim();
    if s.chars().count() <= LISTED_CHARS {
        return s.to_string();
    }
    let end = s
        .char_indices()
        .nth(LISTED_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let cut = &s[..end];
    let cut = cut.rfind(' ').map(|i| &cut[..i]).unwrap_or(cut);
    format!("{cut}...")
}

/// The version of how a note is read: frontmatter, places and disclosure. It goes up with every
/// change to what a note parses to, and an index built under another version rereads every note.
pub const RULES: u32 = 3;
