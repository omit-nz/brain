//! How a note's own dates and declarations change a search: a question that asks for the latest is
//! ordered by when each note was written, and a note another corrects or supersedes says so.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jiff::civil::Date;

use crate::Hit;

const LATEST_OF: [&str; 5] = ["decision", "ruling", "note", "change", "update"];

/// Whether `text` asks for the latest decision, ruling, note, change or update, and what it asks
/// about: the words after the first "on", "about" or "for" that follows, or with none of those
/// every word that follows, if any are left.
pub(crate) fn latest(text: &str) -> Option<Option<String>> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let plain: Vec<String> = words
        .iter()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect();
    let after = (0..plain.len()).find_map(|i| {
        let from = match plain[i].as_str() {
            "latest" | "newest" => i + 1,
            "recent" if i > 0 && plain[i - 1] == "most" => i + 1,
            _ => return None,
        };
        (from..plain.len().min(from + 3))
            .find(|&j| {
                let word = plain[j].strip_suffix('s').unwrap_or(&plain[j]);
                LATEST_OF.contains(&word)
            })
            .map(|j| j + 1)
    })?;
    let from = (after..plain.len())
        .find(|&i| matches!(plain[i].as_str(), "on" | "about" | "for"))
        .map_or(after, |i| i + 1);
    Some(Some(words[from..].join(" ")).filter(|t| !t.is_empty()))
}

/// The first of a note's candidate dates that has come.
pub(crate) fn said(candidates: Option<&str>, today: Date) -> Option<Date> {
    candidates?
        .split(' ')
        .filter_map(|d| d.parse().ok())
        .find(|d| *d <= today)
}

/// Orders the dated hits by when they were written, newest first, in the places dated hits already
/// hold; an undated hit keeps its place. Hits arrive in score order and the sort is stable, so
/// score breaks equal dates.
pub(crate) fn newest_first(hits: &mut [Hit]) {
    let places: Vec<usize> = (0..hits.len())
        .filter(|&i| hits[i].said.is_some())
        .collect();
    let mut dated: Vec<Hit> = places.iter().map(|&i| hits[i].clone()).collect();
    dated.sort_by_key(|hit| std::cmp::Reverse(hit.said));
    for (place, hit) in places.into_iter().zip(dated) {
        hits[place] = hit;
    }
}

/// One note as far as a declaration can name it.
pub(crate) struct Named {
    pub path: String,
    pub id: Option<String>,
    pub said: Option<Date>,
    pub corrects: Option<String>,
    pub supersedes: Option<String>,
    /// Whether the search's view holds it.
    pub seen: bool,
}

/// The note a name answers to: one that two notes answer to names neither.
fn only(found: &[usize]) -> Option<usize> {
    match found {
        [one] => Some(*one),
        _ => None,
    }
}

/// Which note each overtaken note is overtaken by, by path: the newest of those that declare it.
/// Names are resolved over every note, so they mean the same in each view, and a mark stands only
/// when the view holds both its notes.
pub(crate) fn overtakers(notes: &[Named]) -> HashMap<&str, &str> {
    let mut exact: HashMap<String, Vec<usize>> = HashMap::new();
    let mut files: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, n) in notes.iter().enumerate() {
        for name in [Some(&n.path), n.id.as_ref()].into_iter().flatten() {
            exact.entry(name.to_lowercase()).or_default().push(i);
        }
        let path = Path::new(&n.path);
        for name in [path.file_name(), path.file_stem()].into_iter().flatten() {
            let name = name.to_string_lossy().to_lowercase();
            files.entry(name).or_default().push(i);
        }
    }
    let id_or_path = |name: &str| only(exact.get(&name.to_lowercase())?);
    // An id or a path first: a note at the vault root has its file name as its path.
    let named = |name: &str| {
        let key = name.to_lowercase();
        only(exact.get(&key).or_else(|| files.get(&key))?)
    };
    let superseded = |written: &str| {
        named(written.trim()).or_else(|| {
            // `[[x]]`, or the `[x]` the flow list parser leaves of an unquoted one.
            let link = written.trim().strip_prefix('[')?.strip_suffix(']')?;
            let link = link
                .strip_prefix('[')
                .and_then(|l| l.strip_suffix(']'))
                .unwrap_or(link);
            named(link.split(['|', '#']).next()?.trim())
        })
    };
    let mut declared: HashSet<(usize, usize)> = HashSet::new();
    for (over, n) in notes.iter().enumerate() {
        // A correction is by construction the later note, whatever the two dates say.
        if let Some(target) = n.corrects.as_deref().and_then(id_or_path) {
            declared.insert((over, target));
        }
        for target in n
            .supersedes
            .iter()
            .flat_map(|s| s.lines())
            .filter_map(superseded)
        {
            let earlier = matches!((n.said, notes[target].said), (Some(a), Some(b)) if a < b);
            if !earlier {
                declared.insert((over, target));
            }
        }
    }
    let mut marks: HashMap<usize, usize> = HashMap::new();
    for &(over, target) in &declared {
        // A pair naming each other marks neither, and so does a note naming itself.
        let mutual = declared.contains(&(target, over));
        if mutual || !(notes[over].seen && notes[target].seen) {
            continue;
        }
        let rank = |i: usize| (notes[i].said, &notes[i].path);
        let held = marks.entry(target).or_insert(over);
        if rank(over) > rank(*held) {
            *held = over;
        }
    }
    marks
        .into_iter()
        .map(|(target, over)| (notes[target].path.as_str(), notes[over].path.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::latest;

    #[test]
    fn only_the_trigger_phrases_trigger() {
        for yes in [
            "what was the latest decision we made on watering",
            "Newest rulings, please",
            "most recent change to the plan",
            "the latest watering decisions",
            "any latest updates?",
        ] {
            assert!(latest(yes).is_some(), "{yes}");
        }
        for no in [
            "the latest.json file has a note",
            "how does the current vent setup work",
            "current decision on watering",
            "most recent version of the plan",
            "latest watering and planting direction decision",
            "decision on the latest model",
        ] {
            assert_eq!(latest(no), None, "{no}");
        }
    }

    #[test]
    fn the_frame_is_removed_up_to_the_topic() {
        let topic = |q| latest(q).flatten();
        assert_eq!(
            topic("what was the latest decision we made on watering / planting direction for the orchard")
                .as_deref(),
            Some("watering / planting direction for the orchard")
        );
        assert_eq!(
            topic("most recent ruling about Sam").as_deref(),
            Some("Sam")
        );
        assert_eq!(topic("latest decision on"), None);
        assert_eq!(topic("latest decision?"), None);
        assert_eq!(
            topic("latest decision, orchard watering").as_deref(),
            Some("orchard watering")
        );
        assert_eq!(
            topic("latest decision orchard watering planting direction").as_deref(),
            Some("orchard watering planting direction")
        );
    }
}
