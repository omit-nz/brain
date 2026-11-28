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
        ("waiting.md", note(14, "### Still waiting on the bank\n\nx")),
        ("old.md", note(15, OPEN)),
        ("undated.md", "---\nowner: robin\n---\n## Open\n".into()),
        ("closed.md", note(1, "## Done\n\nopen the door, then wait")),
        ("other-words.md", note(1, "## Reopened and opening hours\n")),
        ("code.md", note(1, "```sh\n# open the file\n```")),
        ("hash.md", note(1, "#open is a tag, not a heading")),
        (
            "indented-text.md",
            note(1, "x\n open the door\n\topen it\n# \n"),
        ),
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
        listed[1].starts_with(&days_ago(14)) && listed[1].contains("waiting.md"),
        "{text}"
    );
}

#[test]
fn a_heading_inside_a_code_block_is_not_one() {
    let h = built(&[
        ("tildes.md", note(1, "~~~\n## Open\n~~~")),
        ("mixed.md", note(1, "```\n~~~\n## Open\n```")),
        ("longer.md", note(1, "````\n```\n## Open\n````")),
        ("info.md", note(1, "```\n```sh\n## Open\n```")),
        ("inline.md", note(1, "```x``` is the command\n\n## Open")),
        ("after.md", note(1, "~~~sh\nls\n~~~~\n## Open")),
    ]);
    let text = due(&h);
    assert_eq!(text.matches("<note ").count(), 2, "{text}");
    assert!(
        text.contains("inline.md") && text.contains("after.md"),
        "{text}"
    );
}

#[test]
fn a_heading_is_read_as_markdown_reads_one() {
    let h = built(&[
        ("indented.md", note(1, "x\n\n  ## Open")),
        ("tab.md", note(1, "##\tOpen")),
        // Four spaces make code of the marks, which then holds nothing after it.
        ("after-code.md", note(1, "x:\n\n    ```\n\ntext\n\n## Open")),
        ("after-list.md", note(1, "- ```sh\n  ls\n  ```\n\n## Open")),
        ("in-list.md", note(1, "- ```sh\n  ## Open\n  ```")),
        (
            "in-nested.md",
            note(1, "10. ```\n    x\n    ```\n    ## Open"),
        ),
        // Marks four columns further in than the block's own close nothing.
        ("in-block.md", note(1, "```\n    ```\n## Open\n```")),
    ]);
    let text = due(&h);
    assert_eq!(text.matches("<note ").count(), 4, "{text}");
    assert!(!text.contains("in-"), "{text}");
}

#[test]
fn the_newest_written_is_listed_first() {
    let h = built(&[("a.md", note(9, OPEN)), ("z.md", note(1, OPEN))]);
    let text = due(&h);
    let first = text.lines().find(|l| l.contains("<note ")).unwrap();
    assert!(first.contains("z.md"), "{text}");
}

#[test]
fn a_note_gone_since_the_index_is_not_listed() {
    let h = built(&[("open.md", note(1, OPEN))]);
    std::fs::remove_file(h.vault.path().join("open.md")).unwrap();
    assert_eq!(due(&h), "Nothing of theirs is due.");
}

/// Silence would say nothing is open.
#[test]
fn a_recent_note_that_cannot_be_read_is_counted() {
    let h = built(&[("open.md", note(1, OPEN))]);
    std::fs::write(h.vault.path().join("open.md"), [0xff, 0xfe]).unwrap();
    assert_eq!(
        due(&h),
        "Nothing of theirs is due.\n1 recent note could not be read."
    );
    assert_eq!(
        h.run(&["due"]),
        "nothing of yours is due\n1 recent note could not be read.\n"
    );
    assert_eq!(h.json(&["due", "--json"])["open_unread"], 1);
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
        ("Working-Memory/-proj/copy.md", note(1, OPEN)),
    ]);
    assert_eq!(due(&h), "Nothing of theirs is due.");
}

/// A vault import marks nearly every note private to the company, and they are still the
/// person's own to hear about.
#[test]
fn a_note_the_company_may_not_see_is_still_the_persons_own() {
    let text = note(1, OPEN).replace("owner: robin\n", "owner: robin\nvisibility: private\n");
    let h = built(&[("kept.md", text)]);
    assert!(due(&h).contains("kept.md"));
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
fn five_long_paths_take_at_most_half_the_reply() {
    // Escaped, each `&` is five characters.
    let folder = "&".repeat(200);
    let mut notes: Vec<(String, String)> = (0..40)
        .map(|n| {
            let text = "---\nowner: robin\nreview: 2020-01-01\n---\nx\n".into();
            (format!("d{n}.md"), text)
        })
        .collect();
    notes.extend((1..=5).map(|n| (format!("{folder}/o{n}.md"), note(n, OPEN))));
    let refs: Vec<(&str, &str)> = notes
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let text = due(&indexed(&refs));
    let open = &text[text.find("\nWritten in").unwrap()..];
    assert!(text.len() <= 9_500, "{}", text.len());
    assert!(open.len() <= 4_750, "{}", open.len());
    assert!(open.ends_with(" more not listed."), "{open}");
}

#[test]
fn the_command_counts_the_open_notes_it_leaves_out() {
    let notes: Vec<(String, String)> = (1..=7)
        .map(|n| (format!("n{n}.md"), note(n, OPEN)))
        .collect();
    let refs: Vec<(&str, &str)> = notes
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let h = indexed(&refs);
    let said = h.run(&["due"]);
    assert!(said.contains("n5.md") && !said.contains("n6.md"), "{said}");
    assert!(said.ends_with("\n2 more not listed.\n"), "{said}");
    let out = h.json(&["due", "--json"]);
    assert_eq!(out["open"].as_array().unwrap().len(), 5, "{out}");
    assert_eq!(out["open_more"], 2, "{out}");
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

/// A queue of thousands would flood the terminal, or the AI reading it there.
#[test]
fn the_command_lists_the_oldest_and_counts_the_rest() {
    let notes: Vec<(String, String)> = (0..22)
        .map(|n| {
            let text = format!("---\nowner: robin\nreview: 2020-01-{:02}\n---\nx\n", n + 1);
            (format!("d{n:02}.md"), text)
        })
        .collect();
    let refs: Vec<(&str, &str)> = notes
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let h = indexed(&refs);
    let said = h.run(&["due"]);
    assert_eq!(said.lines().count(), 21, "{said}");
    assert!(
        said.contains("d19.md") && !said.contains("d20.md"),
        "{said}"
    );
    assert!(
        said.ends_with("\n2 more not listed: `brain due --within 14 --all` prints them all.\n"),
        "{said}"
    );
    let all = h.run(&["due", "--all"]);
    assert_eq!(all.lines().count(), 22, "{all}");
    assert!(!all.contains("not listed"), "{all}");
    let json = h.json(&["due", "--json"]);
    assert_eq!(json["due"].as_array().unwrap().len(), 22);
    // The hint printed for a window looking back is one the command takes.
    h.run(&["due", "--within", "-1", "--all"]);
}
