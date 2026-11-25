//! What a search means: a question no note answers is logged as a gap, against the nearest
//! area, and filled once a note answers it.

use brain_index::activity::Activity;
use brain_platform::clock::{Clock, SystemClock};
use brain_report::Event;
use serde_json::Value;

use super::Home;

fn open_gaps(h: &Home) -> Vec<Value> {
    h.json(&["gaps", "--json"])["open"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn search_json_says_gap_and_best_similarity() {
    let h = Home::new();
    h.init();
    h.put("a.md", "orchard pruning schedule\n");
    h.run(&["index"]);
    let found = h.json(&["search", "--json", "pruning"]);
    assert_eq!(found["gap"], false);
    assert_eq!(
        found["best_similarity"],
        Value::Null,
        "no vectors without a runtime"
    );
    assert_eq!(found["hits"][0]["path"], "a.md");
    let missed = h.json(&["search", "--json", "zeppelin"]);
    assert_eq!(missed["gap"], true);
}

#[test]
fn a_search_with_nothing_above_the_floor_logs_a_gap() {
    let h = Home::new();
    h.init();
    h.put("a.md", "orchard pruning schedule\n");
    h.run(&["index"]);
    let out = h.run(&["search", "zeppelin", "hangar"]);
    assert!(
        out.starts_with("no note answers this; logged as a gap"),
        "{out}"
    );
    let gaps = open_gaps(&h);
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0]["query"], "zeppelin hangar");
    assert_eq!(gaps[0]["door"], "cli");
}

#[test]
fn a_search_that_matches_a_word_logs_no_gap() {
    let h = Home::new();
    h.init();
    h.put("a.md", "orchard pruning schedule\n");
    h.run(&["index"]);
    let out = h.run(&["search", "pruning"]);
    assert!(!out.contains("gap"), "{out}");
    assert!(open_gaps(&h).is_empty());
}

#[test]
fn a_gap_names_the_nearest_area() {
    let h = Home::new();
    h.init();
    h.put("clients/acme.md", "the acme account renews in march\n");
    let embed = |args: &[&str]| {
        let out = h
            .brain()
            .env("BRAIN_TEST_EMBEDDER", "1")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    embed(&["index"]);
    embed(&["search", "what is the wifi password"]);
    let gaps = open_gaps(&h);
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert_eq!(gaps[0]["area"], "clients");
}

#[test]
fn a_gap_is_filled_when_a_note_answers_it() {
    let h = Home::new();
    h.init();
    h.put("a.md", "orchard pruning schedule\n");
    h.run(&["index"]);
    h.run(&["search", "zeppelin"]);
    assert_eq!(open_gaps(&h).len(), 1);
    h.put("z.md", "the zeppelin is moored at the north field\n");
    let out = h.run(&["index"]);
    assert!(out.contains("1 gap filled"), "{out}");
    let g = h.json(&["gaps", "--json"]);
    assert_eq!(g["open"].as_array().unwrap().len(), 0);
    assert_eq!(g["filled_last_30_days"], 1);
    assert!(h.run(&["gaps"]).contains("1 filled in the last 30 days"));
    let activity = Activity::open(&h.home.path().join("data/activity.db")).unwrap();
    let today = activity.day(SystemClock.today()).unwrap();
    let filled: u64 = today
        .iter()
        .filter(|(e, _, _)| *e == Event::GapFilled)
        .map(|(_, _, n)| n)
        .sum();
    assert_eq!(filled, 1);
}
