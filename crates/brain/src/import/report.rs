//! What an imported brain needs looked at: notes with no source, an inbox left to age, links to
//! notes that are not there, the same note in two places, and how much is shared.

use std::collections::{HashMap, HashSet};
use std::fmt;

use brain_note::gate;
use brain_note::{Disclosure, Entry, Note, Place, Split, Value, Vault, rel_key};
use jiff::ToSpan;
use jiff::civil::Date;

use crate::inbox::Dest;

/// A list longer than this shows its first lines, then how many more.
const SHOWN: usize = 20;
const INBOX_DAYS: i64 = 14;

#[derive(Debug, Default)]
pub struct Report {
    pub missing_source: Vec<String>,
    pub old_inbox: Vec<String>,
    pub broken_links: Vec<String>,
    pub ghosts: Vec<String>,
    pub shared: usize,
    pub listed: usize,
    pub private: usize,
}

/// Read from the vault itself, so it is never staler than the files. Working memory is not
/// expected to cite a source.
pub fn report(dst: &Vault, today: Date) -> Report {
    let mut r = Report::default();
    let cutoff = today.checked_sub(INBOX_DAYS.days()).unwrap_or(today);
    let mut stems = HashSet::new();
    let mut links = Vec::new();
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    for entry in dst.walk() {
        let Entry::Note { rel, place } = entry else {
            continue;
        };
        let Ok(raw) = dst.read(&rel) else {
            continue;
        };
        let note = Note::parse(&rel, &raw, place);
        let split = Split::parse(&raw);
        let key = rel_key(&rel);
        if let Some(stem) = rel.file_stem() {
            stems.insert(stem.to_string_lossy().to_lowercase());
        }
        if let Some(name) = rel.file_name() {
            names
                .entry(name.to_string_lossy().to_string())
                .or_default()
                .push(key.clone());
        }
        match note.disclosure {
            Disclosure::Shared => r.shared += 1,
            Disclosure::Listed => r.listed += 1,
            Disclosure::Private => r.private += 1,
        }
        if !rel.starts_with(&dst.layout.memory) && !gate::facts(&note, &split).has_source {
            r.missing_source.push(key.clone());
        }
        if place == Place::Inbox {
            let date = split
                .fm
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .and_then(|f| match f.get("date") {
                    Some(Value::Text(d)) => d.trim().parse::<Date>().ok(),
                    _ => None,
                });
            if date.is_some_and(|d| d < cutoff) {
                r.old_inbox.push(key.clone());
            }
        }
        links.extend(note.links.into_iter().map(|t| (key.clone(), t)));
    }
    r.broken_links = links
        .into_iter()
        .filter(|(_, t)| !stems.contains(t))
        .map(|(from, t)| format!("{from} -> [[{t}]]"))
        .collect();
    let inbox = rel_key(&dst.layout.inbox);
    let mut ghosts: Vec<String> = names
        .into_values()
        .filter(|paths| {
            paths.iter().any(|p| p.starts_with(&format!("{inbox}/")))
                && paths
                    .iter()
                    .any(|p| p.starts_with(&format!("{}/", Dest::Permanent.folder())))
        })
        .map(|paths| paths.join(" and "))
        .collect();
    ghosts.sort();
    r.ghosts = ghosts;
    r
}

fn section(f: &mut fmt::Formatter<'_>, title: &str, lines: &[String]) -> fmt::Result {
    writeln!(f, "{title} ({}):", lines.len())?;
    for l in lines.iter().take(SHOWN) {
        writeln!(f, "  {l}")?;
    }
    if lines.len() > SHOWN {
        writeln!(f, "  and {} more", lines.len() - SHOWN)?;
    }
    writeln!(f)
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        section(f, "notes missing a source", &self.missing_source)?;
        section(f, "inbox notes older than 14 days", &self.old_inbox)?;
        section(f, "broken links", &self.broken_links)?;
        section(f, "the same note in the inbox and kept", &self.ghosts)?;
        write!(
            f,
            "visibility: {} shared, {} listed, {} private",
            self.shared, self.listed, self.private
        )
    }
}
