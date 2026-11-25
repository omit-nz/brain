//! What brain counts about its own use: kinds and doors and numbers, never text, and never at the
//! cost of the action counted.

use std::fs;

use serde_json::json;

use super::Home;
use super::mcp::{Mcp, indexed};

const SENTINEL: &str = "ZQX-SENTINEL-7";

/// `brain usage`'s count lines, `kind  door  n`.
fn counts(h: &Home) -> Vec<String> {
    h.run(&["usage"])
        .lines()
        .skip_while(|l| *l != "Today so far:")
        .skip(1)
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

#[test]
fn usage_sending_is_on_by_default() {
    let h = Home::new();
    h.init();
    let config = fs::read_to_string(h.home.path().join("config/config.toml")).unwrap();
    assert!(!config.contains("[usage]"), "{config}");
    let out = h.run(&["usage"]);
    assert!(
        out.starts_with("Usage sending: on, to https://usage.brain.omit.nz/v1/day"),
        "{out}"
    );
}

#[test]
fn usage_off_is_saved() {
    let h = Home::new();
    h.init();
    h.run(&["usage", "off"]);
    assert!(h.run(&["usage"]).starts_with("Usage sending: off"));
    h.run(&["usage", "on"]);
    assert!(h.run(&["usage"]).starts_with("Usage sending: on"));
}

#[test]
fn a_search_counts_a_question_by_door() {
    let h = indexed(&[("a.md", "mailbox notes\n")]);
    h.run(&["search", "mailbox"]);
    let mut m = Mcp::start(&h);
    m.text("search_my_brain", json!({"query": "mailbox"}));
    drop(m);
    let c = counts(&h);
    assert!(c.contains(&"question cli 1".to_string()), "{c:?}");
    assert!(c.contains(&"question claude_code 1".to_string()), "{c:?}");
    let latency: usize = c.iter().filter(|l| l.starts_with("search_")).count();
    assert!(latency >= 1, "a latency bucket per door: {c:?}");
}

#[test]
fn an_answer_from_a_kept_note_is_counted() {
    let h = indexed(&[
        ("3-permanent/a.md", "mailbox notes\n"),
        ("0-inbox/b.md", "kettle\n"),
    ]);
    h.run(&["search", "mailbox"]);
    h.run(&["search", "kettle"]);
    let c = counts(&h);
    assert!(c.contains(&"answer_kept cli 1".to_string()), "{c:?}");
}

#[test]
fn a_gap_is_never_counted_as_an_answer() {
    let h = indexed(&[("3-permanent/a.md", "mailbox notes\n")]);
    h.run(&["search", "zeppelin"]);
    let c = counts(&h);
    assert!(c.contains(&"gap_logged cli 1".to_string()), "{c:?}");
    assert!(!c.iter().any(|l| l.starts_with("answer_kept")), "{c:?}");
}

#[test]
fn inbox_keep_and_skip_are_counted() {
    let h = Home::new();
    h.init();
    h.run(&["add", "one", "--title", "One"]);
    h.run(&["add", "two", "--title", "Two"]);
    h.run(&["inbox", "keep", "0-inbox/one.md"]);
    h.run(&["inbox", "skip", "0-inbox/two.md"]);
    let c = counts(&h);
    for line in [
        "capture_written cli 2",
        "capture_kept cli 1",
        "capture_skipped cli 1",
    ] {
        assert!(c.contains(&line.to_string()), "{line}: {c:?}");
    }
}

#[test]
fn counts_never_hold_text() {
    let h = indexed(&[(
        &format!("{SENTINEL}/{SENTINEL}.md"),
        &format!("---\ntitle: {SENTINEL}\n---\n{SENTINEL} body\n"),
    )]);
    h.run(&["search", SENTINEL]);
    h.run(&["search", &format!("{SENTINEL} nowhere zeppelin")]);
    h.run(&["add", SENTINEL, "--title", SENTINEL]);
    h.run(&["correct", &format!("{SENTINEL}/{SENTINEL}.md"), SENTINEL]);
    let mut m = Mcp::start(&h);
    m.text("search_my_brain", json!({"query": SENTINEL}));
    m.text("capture", json!({"text": SENTINEL, "title": SENTINEL}));
    drop(m);
    let db = rusqlite::Connection::open(h.home.path().join("data/activity.db")).unwrap();
    let columns: Vec<String> = db
        .prepare("SELECT name FROM pragma_table_info('counts') ORDER BY cid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, vec!["day", "kind", "door", "n"]);
    let rows: Vec<String> = db
        .prepare("SELECT day || ' ' || kind || ' ' || door || ' ' || n FROM counts")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(rows.len() >= 5, "{rows:?}");
    for row in &rows {
        assert!(!row.contains(SENTINEL), "{row}");
    }
}

#[test]
fn a_count_that_cannot_be_written_never_fails_the_action() {
    let h = indexed(&[("a.md", "mailbox\n")]);
    let store = h.home.path().join("data/activity.db");
    let _ = fs::remove_file(&store);
    fs::create_dir_all(&store).unwrap();
    for args in [
        vec!["search", "mailbox"],
        vec!["search", "zeppelin"],
        vec!["add", "still saved"],
    ] {
        let out = h.brain().args(&args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("was not saved"),
            "{args:?}"
        );
    }
    let mut m = Mcp::start(&h);
    let (text, err) = m.text("capture", json!({"text": "kept anyway"}));
    assert!(!err, "{text}");
    assert_eq!(h.inbox().len(), 2);
}
