//! What brain keeps of each tool call an AI makes: what was asked and what came back, never the
//! words of a note.

use std::fs;
use std::time::{Duration, Instant};

use brain_platform::clock::{Clock, SystemClock};
use brain_reflex::TRACES;
use brain_reflex::trace::{Run, Store};
use serde_json::{Value, json};

use super::Home;
use super::mcp::{Mcp, indexed};

const ID: &str = "0195f2e0-0000-7000-8000-000000000001";

/// One note of robin's, due, whose body and title appear in no question asked of it.
fn one_note() -> Home {
    indexed(&[(
        "plan.md",
        &format!(
            "---\nid: {ID}\ntitle: Perihelion plan\nowner: robin\nreview: 2020-01-01\n---\nzanzibar \
             quokka waters the orchard\n"
        ),
    )])
}

fn calls(h: &Home) -> Vec<Value> {
    h.json(&["calls", "--json"]).as_array().unwrap().clone()
}

fn db(h: &Home) -> rusqlite::Connection {
    rusqlite::Connection::open(h.home.path().join("data").join(TRACES)).unwrap()
}

#[test]
fn a_call_is_recorded_with_what_was_asked_and_what_came_back() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    let asked = json!({"query": "orchard", "limit": 3});
    let (text, err) = m.text("search_my_brain", asked.clone());
    assert!(!err && text.contains(ID), "{text}");
    let (due, _) = m.text("due", json!({"within_days": 7}));
    let held = calls(&h);
    assert_eq!(held.len(), 2, "{held:?}");
    // Newest first.
    assert_eq!(held[0]["tool"], "due");
    assert_eq!(held[0]["args"], json!({"within_days": 7}));
    assert_eq!(held[0]["reply_bytes"], due.len());
    assert_eq!(held[0]["notes"], json!([ID]));
    assert_eq!(held[1]["tool"], "search_my_brain");
    assert_eq!(held[1]["args"], asked);
    assert_eq!(held[1]["reply_bytes"], text.len());
    assert_eq!(held[1]["notes"], json!([ID]));
    for call in &held {
        assert_eq!(call["door"], "claude_code", "{call}");
        assert_eq!(call["error"], false, "{call}");
        let at: jiff::Timestamp = call["at"].as_str().unwrap().parse().unwrap();
        let ago = SystemClock.now().duration_since(at).as_secs();
        assert!((0..600).contains(&ago), "{call}");
    }
}

#[test]
fn no_note_text_is_kept() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    let ok = |m: &mut Mcp, tool: &str, args: Value| {
        let (text, err) = m.text(tool, args);
        assert!(!err, "{tool}: {text}");
        text
    };
    ok(&mut m, "search_my_brain", json!({"query": "orchard"}));
    ok(&mut m, "search_company", json!({"query": "orchard"}));
    ok(&mut m, "due", json!({}));
    ok(
        &mut m,
        "capture",
        json!({"text": "obsidian wombat", "title": "marmalade heron", "kind": "rule"}),
    );
    ok(
        &mut m,
        "correct",
        json!({"target": "plan.md", "correction": "turquoise badger"}),
    );
    ok(
        &mut m,
        "room_post",
        json!({"text": "saffron lynx", "title": "cobalt newt", "as": "tester"}),
    );
    let read = ok(&mut m, "room_new", json!({"as": "other"}));
    assert!(read.contains("saffron lynx"), "{read}");
    ok(&mut m, "room_list", json!({}));
    drop(m);
    let held = calls(&h);
    assert_eq!(held.len(), 8, "{held:?}");
    let of = |tool: &str| held.iter().find(|c| c["tool"] == tool).unwrap();
    assert_eq!(
        of("capture")["args"],
        json!({"text": "15 characters", "title": "15 characters", "kind": "rule"})
    );
    assert_eq!(
        of("correct")["args"],
        json!({"target": "7 characters", "correction": "16 characters"})
    );
    assert_eq!(
        of("room_post")["args"],
        json!({"room": null, "text": "12 characters", "title": "11 characters", "as": "tester"})
    );
    // The post another session read is named, so retracting it takes the call too.
    assert_eq!(of("room_new")["notes"].as_array().unwrap().len(), 1);
    // The files themselves, the log beside them included.
    let mut bytes = Vec::new();
    for entry in fs::read_dir(h.home.path().join("data")).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(TRACES)
        {
            bytes.extend(fs::read(path).unwrap_or_default());
        }
    }
    let kept = String::from_utf8_lossy(&bytes);
    assert!(kept.contains("search_my_brain"), "nothing was recorded");
    for words in [
        "zanzibar",
        "Perihelion",
        "obsidian wombat",
        "marmalade",
        "turquoise",
        "saffron",
        "cobalt",
    ] {
        assert!(!kept.contains(words), "{words} was kept");
    }
}

#[test]
fn a_question_too_long_to_be_one_is_kept_as_its_length() {
    let h = one_note();
    let (text, err) = Mcp::start(&h).text("search_my_brain", json!({"query": "x".repeat(2_001)}));
    assert!(err, "{text}");
    let held = calls(&h);
    assert_eq!(
        held[0]["args"],
        json!({"query": "2001 characters", "limit": null})
    );
    assert_eq!(held[0]["error"], true);
    assert_eq!(held[0]["reply_bytes"], text.len());
}

#[test]
fn a_removed_note_takes_the_calls_that_named_it() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    m.text("search_my_brain", json!({"query": "orchard"}));
    m.text("room_list", json!({}));
    drop(m);
    fs::remove_file(h.vault.path().join("plan.md")).unwrap();
    h.run(&["index"]);
    let held = calls(&h);
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0]["tool"], "room_list");
}

#[test]
fn a_call_is_dropped_after_30_days() {
    let h = one_note();
    Mcp::start(&h).text("room_list", json!({}));
    let aged = "UPDATE runs SET started_ns = started_ns - ?1";
    let day = 24 * 60 * 60 * 1_000_000_000_i64;
    db(&h).execute(aged, [29 * day]).unwrap();
    assert_eq!(calls(&h).len(), 1);
    db(&h).execute(aged, [2 * day]).unwrap();
    assert_eq!(h.run(&["calls"]), "no calls recorded\n");
}

#[test]
fn the_command_prints_the_last_calls_newest_first() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    m.text("room_list", json!({}));
    m.text("search_my_brain", json!({"query": "orchard"}));
    let (due, _) = m.text("due", json!({"within_days": 7}));
    drop(m);
    // Runs that are no tool call are not calls.
    let store = Store::open(&h.home.path().join("data").join(TRACES)).unwrap();
    Run::record(&store, "memory follow").finish().unwrap();
    let said = h.run(&["calls", "--last", "2"]);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 2, "{said}");
    assert!(
        lines[0].ends_with(&format!(
            "claude_code  due  {{\"within_days\":7}}  {} bytes, 1 note",
            due.len()
        )),
        "{said}"
    );
    assert!(lines[1].contains("search_my_brain"), "{said}");
    assert_eq!(h.run(&["calls"]).lines().count(), 3);
}

#[test]
fn nothing_recorded_makes_no_trace_file() {
    let h = one_note();
    assert_eq!(h.run(&["calls"]), "no calls recorded\n");
    assert!(!h.home.path().join("data").join(TRACES).exists());
}

/// Another writer holding the file is waited on for five seconds; a reply is not.
#[test]
fn a_busy_trace_store_never_holds_a_reply() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    m.text("room_list", json!({}));
    let held = db(&h);
    held.execute_batch("BEGIN IMMEDIATE").unwrap();
    let asked = Instant::now();
    let (text, err) = m.text("room_list", json!({}));
    assert!(!err, "{text}");
    assert!(
        asked.elapsed() < Duration::from_millis(3_500),
        "{:?}",
        asked.elapsed()
    );
}

/// A captured note's file is named from its own words, so a path is words of a note too.
#[test]
fn a_corrected_note_is_kept_as_a_source_and_never_by_name() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    let (said, _) = m.text("capture", json!({"text": "obsidian wombat owes marmalade"}));
    let path = said
        .split("path=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(path.contains("obsidian-wombat"), "{said}");
    h.run(&["index"]);
    let (said, err) = m.text("correct", json!({"target": path, "correction": "no"}));
    assert!(!err, "{said}");
    drop(m);
    let held = calls(&h);
    assert_eq!(
        held[0]["args"]["target"],
        format!("{} characters", path.len())
    );
    // The correction and the note it corrects.
    assert_eq!(held[0]["notes"].as_array().unwrap().len(), 2, "{held:?}");
    h.run(&["index"]);
    fs::remove_file(h.vault.path().join(path)).unwrap();
    h.run(&["index"]);
    assert_eq!(calls(&h), Vec::<Value>::new(), "both calls named it");
}

/// A row whose reply was never written still says which notes it must go with.
#[test]
fn a_calls_notes_are_written_with_it() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    m.text("room_list", json!({}));
    db(&h)
        .execute_batch(
            "CREATE TRIGGER no_end BEFORE UPDATE ON spans BEGIN SELECT RAISE(ABORT, 'x'); END",
        )
        .unwrap();
    m.text("search_my_brain", json!({"query": "orchard"}));
    drop(m);
    assert_eq!(calls(&h)[0]["notes"], json!([ID]));
}

#[test]
fn a_client_that_quits_after_the_reply_still_leaves_the_call() {
    let h = one_note();
    let mut m = Mcp::start(&h);
    m.text("room_list", json!({}));
    let held = db(&h);
    held.execute_batch("BEGIN IMMEDIATE").unwrap();
    m.text("room_list", json!({}));
    m.hang_up();
    // Long enough for a server that does not wait for its record to be gone.
    std::thread::sleep(Duration::from_millis(500));
    held.execute_batch("COMMIT").unwrap();
    m.exited();
    assert_eq!(calls(&h).len(), 2);
}

#[test]
fn a_question_of_2000_characters_is_kept_as_asked() {
    let h = one_note();
    let query = "x".repeat(2_000);
    Mcp::start(&h).text("search_my_brain", json!({"query": query}));
    assert_eq!(calls(&h)[0]["args"]["query"], query);
}

#[test]
fn a_failed_call_and_two_notes_print_as_such() {
    let h = indexed(&[
        (
            "a.md",
            "---\nid: 0195f2e0-0000-7000-8000-00000000000a\nowner: robin\nreview: 2020-01-01\n---\nx\n",
        ),
        (
            "b.md",
            "---\nid: 0195f2e0-0000-7000-8000-00000000000b\nowner: robin\nreview: 2020-01-01\n---\ny\n",
        ),
    ]);
    let mut m = Mcp::start(&h);
    m.text("due", json!({}));
    let (_, err) = m.text("correct", json!({"target": "no.md", "correction": "x"}));
    assert!(err);
    drop(m);
    let said = h.run(&["calls"]);
    let lines: Vec<&str> = said.lines().collect();
    assert!(lines[0].ends_with("bytes, 0 notes, error"), "{said}");
    assert!(lines[1].ends_with("bytes, 2 notes"), "{said}");
}

#[test]
fn a_trace_store_that_cannot_be_written_leaves_the_reply_alone() {
    let h = one_note();
    fs::create_dir_all(h.home.path().join("data").join(TRACES)).unwrap();
    let (text, err) = Mcp::start(&h).text("room_list", json!({}));
    assert!(!err && text.starts_with("No rooms yet"), "{text}");
    let said = fs::read_to_string(h.home.path().join("mcp.err")).unwrap();
    assert!(said.contains("not traced"), "{said}");
}

#[test]
fn the_count_asked_for_is_of_calls() {
    let h = one_note();
    Mcp::start(&h).text("room_list", json!({}));
    // A run cut short before its call was written is no call.
    let store = Store::open(&h.home.path().join("data").join(TRACES)).unwrap();
    Run::record(&store, "mcp").finish().unwrap();
    assert_eq!(h.run(&["calls", "--last", "1"]).lines().count(), 1);
    assert!(h.run(&["calls", "--last", "1"]).contains("room_list"));
    h.brain().args(["calls", "--last", "0"]).assert().failure();
}
