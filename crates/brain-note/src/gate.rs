//! The mechanical half of learning-loop's promote gate: facts a program can count. The judged half
//! (depth, sourcing, linking, voice, atomicity, source integrity) comes from the promote judge;
//! until it has scored a note, the gate never promotes it past fleeting.

use crate::frontmatter::{Split, Value};
use crate::note::Note;
use crate::prose::prose;

/// learning-loop's markers of a claim not yet settled.
const MARKERS: &[&str] = &[
    "[unresolved]",
    "[unverified]",
    "[not in abstract]",
    "[not in source]",
    "[needs verification]",
    "[citation needed]",
];

/// Keys every note brain writes carries; one missing means the note came from elsewhere.
const KEYS: &[&str] = &["id", "date", "owner", "review"];

/// A synthesis with at least this many links is a map.
const MAP_LINKS: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateFacts {
    pub markers: Vec<String>,
    pub wikilinks: usize,
    pub has_source: bool,
    pub body_lines: usize,
    pub missing_keys: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Inbox,
    Fleeting,
    Permanent,
    Maps,
}

/// The markers in `body`, in order, skipping fenced code.
fn markers(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in prose(body) {
        let lower = line.to_lowercase();
        let mut found: Vec<(usize, &str)> = MARKERS
            .iter()
            .flat_map(|m| lower.match_indices(m).map(|(i, _)| (i, *m)))
            .collect();
        found.sort();
        out.extend(found.into_iter().map(|(_, m)| m.to_string()));
    }
    out
}

pub fn facts(note: &Note, split: &Split<'_>) -> GateFacts {
    let fm = split.fm.as_ref().ok().and_then(Option::as_ref);
    let has_source = match fm.and_then(|f| f.get("source")) {
        Some(Value::Text(t)) => !t.trim().is_empty(),
        Some(Value::List(v)) => !v.is_empty(),
        _ => false,
    };
    GateFacts {
        markers: markers(&note.body),
        wikilinks: note.links.len(),
        has_source,
        body_lines: note.body.lines().filter(|l| !l.trim().is_empty()).count(),
        missing_keys: KEYS
            .iter()
            .copied()
            .filter(|k| fm.and_then(|f| f.get(k)).is_none())
            .collect(),
    }
}

/// Where the note belongs. A marker keeps it fleeting whatever its score; an unjudged note never
/// goes past the inbox.
pub fn route(facts: &GateFacts, judged: Option<u8>, synthesis: bool) -> Route {
    if !facts.markers.is_empty() {
        return Route::Fleeting;
    }
    match judged {
        None => Route::Inbox,
        Some(6..) if synthesis && facts.wikilinks >= MAP_LINKS => Route::Maps,
        Some(6..) => Route::Permanent,
        Some(3..=5) => Route::Fleeting,
        Some(_) => Route::Inbox,
    }
}
