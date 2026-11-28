//! What `due` says beside review dates: the recent notes with something left open, which brain
//! cannot tell are still open.

use brain_platform::clock::{Clock, SystemClock};
use jiff::ToSpan;
use serde_json::json;

use super::Home;
use super::mcp::{Mcp, indexed};

const HEADING: &str = "Written in the last 14 days with something left open. brain cannot tell \
                       whether it has been settled since:";

fn days_ago(n: i64) -> String {
    SystemClock
        .today()
        .checked_sub(n.days())
        .unwrap()
        .to_string()
}

/// A note of robin's written `ago` days ago, holding `body`.
fn note(ago: i64, body: &str) -> String {
    format!("---\nowner: robin\ndate: {}\n---\n{body}\n", days_ago(ago))
}

const OPEN: &str = "## Open for Robin\n\n- a thing";

fn due(h: &Home) -> String {
    let (text, err) = Mcp::start(h).text("due", json!({}));
    assert!(!err, "{text}");
    text
}

fn built(notes: &[(&str, String)]) -> Home {
    let refs: Vec<(&str, &str)> = notes.iter().map(|(a, b)| (*a, b.as_str())).collect();
    indexed(&refs)
}

#[test]
fn recent_open_notes_are_listed_with_their_day() {
    let h = built(&[
        ("open.md", note(1, OPEN)),
        ("waiting.md", note(13, "### Still waiting on the bank\n\nx")),
        ("old.md", note(15, OPEN)),
        ("undated.md", "---\nowner: robin\n---\n## Open\n".into()),
        ("closed.md", note(1, "## Done\n\nopen the door, then wait")),
        ("other-words.md", note(1, "## Reopened and opening hours\n")),
        ("code.md", note(1, "```sh\n# open the file\n```")),
        ("hash.md", note(1, "#open is a tag, not a heading")),
    ]);
    let text = due(&h);
    assert!(
        text.starts_with(&format!("Nothing of theirs is due.\n{HEADING}")),
        "{text}"
    );
    let listed: Vec<&str> = text.lines().filter(|l| l.contains("<note ")).collect();
    assert_eq!(listed.len(), 2, "{text}");
    assert!(
        listed[0].starts_with(&days_ago(1)) && listed[0].contains("open.md"),
        "{text}"
    );
    assert!(
        listed[1].starts_with(&days_ago(13)) && listed[1].contains("waiting.md"),
        "{text}"
    );
}

#[test]
fn a_note_another_corrects_or_supersedes_is_not_listed() {
    let h = built(&[
        ("corrected.md", note(2, OPEN)),
        ("superseded.md", note(2, OPEN)),
        (
            "fix.md",
            note(1, "x").replace("owner: robin\n", "owner: robin\ncorrects: corrected.md\n"),
        ),
        (
            "new.md",
            note(1, "x").replace(
                "owner: robin\n",
                "owner: robin\nsupersedes: superseded.md\n",
            ),
        ),
        ("still.md", note(2, OPEN)),
    ]);
    let text = due(&h);
    assert!(text.contains("still.md"), "{text}");
    assert!(
        !text.contains("corrected.md") && !text.contains("superseded.md"),
        "{text}"
    );
}

/// The loop the listing relies on: say it is settled once and it is gone.
#[test]
fn correcting_a_listed_note_takes_it_off() {
    let h = built(&[("open.md", note(2, OPEN))]);
    let mut m = Mcp::start(&h);
    let (text, err) = m.text(
        "correct",
        json!({"target": "open.md", "correction": "Done."}),
    );
    assert!(!err, "{text}");
    h.run(&["index"]);
    assert_eq!(due(&h), "Nothing of theirs is due.");
}

#[test]
fn only_the_persons_own_kept_notes_are_listed() {
    let h = built(&[
        ("theirs.md", note(1, OPEN).replace("robin", "sam")),
        ("private/mine.md", note(1, OPEN)),
        ("0-inbox/said.md", note(1, OPEN)),
    ]);
    assert_eq!(due(&h), "Nothing of theirs is due.");
}

#[test]
fn five_are_listed_and_the_rest_counted() {
    let notes: Vec<(String, String)> = (1..=7)
        .map(|n| (format!("n{n}.md"), note(n, OPEN)))
        .collect();
    let refs: Vec<(&str, &str)> = notes
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let text = due(&indexed(&refs));
    assert_eq!(text.matches("<note ").count(), 5, "{text}");
    assert!(text.contains("n5.md") && !text.contains("n6.md"), "{text}");
    assert!(text.ends_with("\n2 more not listed."), "{text}");
}

#[test]
fn a_full_review_queue_leaves_room_for_the_open_ones_inside_the_budget() {
    // Escaped, each `&` is five characters.
    let long = "&".repeat(900);
    let mut notes: Vec<(String, String)> = (0..40)
        .map(|n| {
            let text =
                format!("---\ntitle: Due {long}\nowner: robin\nreview: 2020-01-01\n---\nx\n");
            (format!("d{n}.md"), text)
        })
        .collect();
    notes.extend((1..=5).map(|n| {
        let text = note(n, OPEN).replace(
            "owner: robin\n",
            &format!("owner: robin\ntitle: Open {long}\n"),
        );
        (format!("o{n}.md"), text)
    }));
    let refs: Vec<(&str, &str)> = notes
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let text = due(&indexed(&refs));
    assert!(text.len() <= 9_500, "{}", text.len());
    assert!(text.starts_with("Due for review: 40 now"), "{text}");
    assert!(text.contains(HEADING), "{text}");
    assert!(text.contains("o1.md"), "{text}");
    assert!(text.contains("d0.md"), "{text}");
}

#[test]
fn the_command_prints_the_open_notes_too() {
    let h = built(&[
        ("open.md", note(1, OPEN)),
        (
            "due.md",
            "---\nowner: robin\nreview: 2020-01-01\n---\nx\n".into(),
        ),
    ]);
    let said = h.run(&["due"]);
    assert!(said.contains("due.md") && said.contains(HEADING), "{said}");
    assert!(
        said.contains(&format!("{}  open.md", days_ago(1))),
        "{said}"
    );
    let out = h.json(&["due", "--json"]);
    assert_eq!(out["due"][0]["path"], "due.md", "{out}");
    assert_eq!(out["open"][0]["path"], "open.md", "{out}");
    assert_eq!(out["open"][0]["said"], days_ago(1).as_str(), "{out}");
    assert_eq!(out["open_more"], 0, "{out}");
    let empty = built(&[]);
    assert_eq!(empty.run(&["due"]), "nothing of yours is due\n");
}
