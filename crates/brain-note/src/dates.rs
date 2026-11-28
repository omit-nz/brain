//! When a note says it was written. A file's time is never used: an import resets it.

use jiff::civil::Date;

/// A frontmatter date this many days after the file name's was stamped by a bulk edit.
const BULK_DAYS: i32 = 7;

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// A note's dates in the order they are tried: its frontmatter's `date` or `created`, the one in
/// its file name, and the first in its first line of text; for a copy of an AI tool's memory, then
/// the first in its second line. A search takes the first that has come.
pub(crate) fn candidates(
    frontmatter: Option<&str>,
    stem: &str,
    body: &str,
    memory: bool,
) -> Vec<Date> {
    let written = frontmatter.and_then(|v| v.get(..10)?.parse::<Date>().ok());
    let named = first_iso(&stem.replace('_', "-"));
    let mut lines = body.lines().filter(|l| !l.trim().is_empty());
    let said = lines.next().and_then(first_date);
    // A copy opens with the memory's description; the memory's own first line comes after it.
    let own = lines.next().filter(|_| memory).and_then(first_date);
    let bulk = matches!((written, named), (Some(w), Some(n)) if (w - n).get_days() > BULK_DAYS);
    let order = if bulk {
        [named, written, said]
    } else {
        [written, named, said]
    };
    order.into_iter().chain([own]).flatten().collect()
}

/// The first date of a line, written `2026-11-27` or `27 Nov 2026`.
fn first_date(line: &str) -> Option<Date> {
    let words: Vec<&str> = line
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .filter(|w| !w.is_empty())
        .collect();
    (0..words.len()).find_map(|i| first_iso(words[i]).or_else(|| spelled(&words[i..])))
}

/// The first `YYYY-MM-DD` in `text` that no digit runs into.
fn first_iso(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    (0..text.len()).find_map(|at| {
        let found = text.get(at..at + 10)?;
        let digit = |i: usize| bytes.get(i).is_some_and(u8::is_ascii_digit);
        let alone = !(at > 0 && digit(at - 1)) && !digit(at + 10);
        let shaped = found.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
        (alone && shaped).then(|| found.parse().ok())?
    })
}

/// `27 Nov 2026`, the month by its first three letters or in full.
fn spelled(words: &[&str]) -> Option<Date> {
    let [day, month, year, ..] = words else {
        return None;
    };
    let month = month.to_ascii_lowercase();
    let at = MONTHS.iter().position(|m| m.starts_with(&month))?;
    if month.len() < 3 || day.len() > 2 || year.len() != 4 {
        return None;
    }
    Date::new(year.parse().ok()?, at as i8 + 1, day.parse().ok()?).ok()
}
