//! New notes: a capture lands in the inbox and a correction is a rule note that names the note it
//! corrects. The CLI and the MCP server both write through here.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;
use brain_index::Index;
use brain_note::{Frontmatter, Kind, MAX_NOTE_BYTES, Split, Vault, rel_key};
use brain_platform::fs::{Privacy, create_new};
use jiff::ToSpan;
use jiff::civil::Date;

use crate::config::REVIEW_DAYS;

pub struct Capture<'a> {
    pub text: &'a str,
    pub title: Option<&'a str>,
    pub kind: Option<Kind>,
    /// The id, or the path, of the note this one corrects.
    pub corrects: Option<&'a str>,
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

/// Writes `c` as a new inbox note owned by `me` and returns its vault path. Its name is the
/// slug of its title (or text), then `-1`, `-2` and on, taking the first name no file holds, so
/// two captures at one moment never overwrite each other.
pub fn write(vault: &Vault, me: &str, today: Date, c: &Capture<'_>) -> anyhow::Result<String> {
    let review = today.checked_add(REVIEW_DAYS.days())?;
    let mut fm = Frontmatter::new_with("id", &uuid::Uuid::now_v7().to_string(), "\n")
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut fields = Vec::new();
    if let Some(t) = c.title {
        fields.push(("title", t.to_string()));
    }
    fields.push(("date", today.to_string()));
    fields.push(("owner", me.to_string()));
    fields.push(("review", review.to_string()));
    if let Some(k) = c.kind {
        fields.push(("kind", k.as_str().to_string()));
    }
    if let Some(target) = c.corrects {
        fields.push(("corrects", target.to_string()));
    }
    for (key, value) in fields {
        fm.set(key, &value)
            .map_err(|_| anyhow::anyhow!("{key} cannot hold a line break"))?;
    }
    let body = format!("{}\n", c.text.trim_end());
    let note = Split {
        bom: false,
        fm: Ok(Some(fm)),
        body: &body,
    }
    .render();
    // The vault walk skips a note this large, so it would never be found.
    anyhow::ensure!(
        note.len() as u64 <= MAX_NOTE_BYTES,
        "a capture is at most 2 MiB"
    );
    let base = slug(c.title.unwrap_or(c.text));
    let inbox = &vault.layout.inbox;
    for n in 0.. {
        let name = match n {
            0 => format!("{base}.md"),
            n => format!("{base}-{n}.md"),
        };
        let rel = inbox.join(name);
        match create_new(&vault.root, &rel, note.as_bytes(), Privacy::Normal) {
            Ok(()) => return Ok(rel_key(&rel)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).with_context(|| format!("writing {}", rel_key(&rel))),
        }
    }
    unreachable!("a name is always free")
}

/// Writes a rule note correcting the indexed note `target` (an id or a vault path) with `text`;
/// the target itself is never edited.
pub fn correct(
    vault: &Vault,
    index: &Index,
    me: &str,
    today: Date,
    target: &str,
    text: &str,
) -> anyhow::Result<String> {
    let noted = index
        .lookup(target)?
        .with_context(|| format!("no note is {target}"))?;
    let stem = Path::new(&noted.path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let corrects = noted
        .note_uuid
        .map_or_else(|| noted.path.clone(), |u| u.to_string());
    let title = format!("Correction: {}", noted.title);
    write(
        vault,
        me,
        today,
        &Capture {
            text: &format!("{}\n\nCorrects [[{stem}]].", text.trim_end()),
            title: Some(&title),
            kind: Some(Kind::Rule),
            corrects: Some(&corrects),
        },
    )
}
