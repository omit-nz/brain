//! How a note's own dates and declarations change a search: a question that asks for the latest is
//! ordered by when each note was written, and a note another corrects or supersedes says so.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jiff::civil::Date;

use crate::Hit;

const LATEST_OF: [&str; 5] = ["decision", "ruling", "note", "change", "update"];

/// Whether `text` asks for the latest decision, ruling, note, change or update, and what it asks
/// about: the words after the first "on", "about" or "for" that follows, if any.
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
    let topic = (after..plain.len())
        .find(|&i| matches!(plain[i].as_str(), "on" | "about" | "for"))
        .map(|i| words[i + 1..].join(" "))
        .filter(|t| !t.is_empty());
    Some(topic)
}

/// The first of a note's candidate dates that has come.
pub(crate) fn said(candidates: Option<&str>, today: Date) -> Option<Date> {
    candidates?
        .split(' ')
        .filter_map(|d| d.parse().ok())
        .find(|d| *d <= today)
}

/// Orders the dated hits by when they were written, newest first, in the places dated hits already
/// hold; an undated hit keeps its place.
pub(crate) fn newest_first(hits: &mut [Hit]) {
    let places: Vec<usize> = (0..hits.len())
        .filter(|&i| hits[i].said.is_some())
        .collect();
    let mut dated: Vec<Hit> = places.iter().map(|&i| hits[i].clone()).collect();
    dated.sort_by(|a, b| b.said.cmp(&a.said).then(b.score.total_cmp(&a.score)));
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
}

/// Which note each overtaken note is overtaken by, by path: the newest of those that declare it.
pub(crate) fn overtakers(notes: &[Named]) -> HashMap<&str, &str> {
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, n) in notes.iter().enumerate() {
        let path = Path::new(&n.path);
        let file = path.file_name().map(|f| f.to_string_lossy());
        let stem = path.file_stem().map(|f| f.to_string_lossy());
        for name in [file, stem].into_iter().flatten() {
            by_name.entry(name.to_lowercase()).or_default().push(i);
        }
        by_name.entry(n.path.to_lowercase()).or_default().push(i);
        if let Some(id) = &n.id {
            by_name.entry(id.to_lowercase()).or_default().push(i);
        }
    }
    // A name two notes answer to names neither. A note at the vault root has its file name as its
    // path, so one note can be listed twice.
    let resolve = |name: &str| -> Option<usize> {
        let name = name.trim_matches(|c: char| "[]\"' ".contains(c));
        let name = name.split(['|', '#']).next()?.to_lowercase();
        let found = by_name.get(&name)?;
        found.iter().all(|i| *i == found[0]).then_some(found[0])
    };
    let mut declared: HashSet<(usize, usize)> = HashSet::new();
    for (over, n) in notes.iter().enumerate() {
        // A correction is by construction the later note, whatever the two dates say.
        if let Some(target) = n.corrects.as_deref().and_then(resolve) {
            declared.insert((over, target));
        }
        for target in n
            .supersedes
            .iter()
            .flat_map(|s| s.lines())
            .filter_map(resolve)
        {
            let earlier = matches!((n.said, notes[target].said), (Some(a), Some(b)) if a < b);
            if !earlier {
                declared.insert((over, target));
            }
        }
    }
    let mut marks: HashMap<usize, usize> = HashMap::new();
    for &(over, target) in &declared {
        if over == target || declared.contains(&(target, over)) {
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
            "what was the latest decision we made on pricing",
            "Newest rulings, please",
            "most recent change to the plan",
            "the latest pricing decisions",
            "any latest updates?",
        ] {
            assert!(latest(yes).is_some(), "{yes}");
        }
        for no in [
            "the latest.json file has a note",
            "how does the current seat setup work",
            "current decision on pricing",
            "most recent version of the plan",
            "latest pricing and marketing direction decision",
            "decision on the latest model",
        ] {
            assert_eq!(latest(no), None, "{no}");
        }
    }

    #[test]
    fn the_frame_is_removed_up_to_the_topic() {
        let topic = |q| latest(q).flatten();
        assert_eq!(
            topic("what was the latest decision we made on pricing / marketing direction for omit")
                .as_deref(),
            Some("pricing / marketing direction for omit")
        );
        assert_eq!(
            topic("most recent ruling about Jake").as_deref(),
            Some("Jake")
        );
        assert_eq!(topic("latest decision on"), None);
        assert_eq!(topic("latest decision, omit pricing"), None);
    }
}
