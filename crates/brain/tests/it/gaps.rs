//! What a search means: a question no note answers is logged as a gap, against the nearest
//! area, and filled once a note answers it.

use brain_index::activity::Activity;
use brain_platform::clock::{Clock, SystemClock};
use brain_report::Event;
use serde_json::{Value, json};

use super::Home;

fn open_gaps(h: &Home) -> Vec<Value> {
    h.json(&["gaps", "--json"])["open"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn search_json_says_gap() {
    let h = Home::new();
    h.init();
    h.put("a.md", "orchard pruning schedule\n");
    h.run(&["index"]);
    let found = h.json(&["search", "--json", "pruning"]);
    assert_eq!(found["gap"], false);
    assert_eq!(found["hits"][0]["path"], "a.md");
    let missed = h.json(&["search", "--json", "zeppelin"]);
    assert_eq!(missed["gap"], true);
}

/// With ONNX Runtime, as every install has it, `brain search` reranks: a note that shares the
/// question's words but does not answer it is not listed, and with only such notes it is a gap.
#[test]
fn search_with_the_runtime_lists_only_what_answers() {
    let ort = std::env::var_os("ORT_DYLIB_PATH")
        .expect("set ORT_DYLIB_PATH (scripts/fetch-ort.sh); CI does");
    let h = Home::new();
    h.init();
    h.put(
        "recipe.md",
        "French toast: bread soaked in eggs and milk, fried in butter, a breakfast in France.\n",
    );
    h.run(&["index"]);
    let search = || {
        let out = h
            .brain()
            .env("ORT_DYLIB_PATH", &ort)
            .args(["search", "--json", "what is the capital of France"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let missed = search();
    assert_eq!(missed["gap"], true, "{missed}");
    assert_eq!(missed["hits"], json!([]), "{missed}");
    h.put(
        "answer.md",
        "Paris is the capital and largest city of France.\n",
    );
    h.run(&["index"]);
    let found = search();
    assert_eq!(found["gap"], false, "{found}");
    let paths: Vec<&str> = found["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["answer.md"], "{found}");
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

/// The daemon's and `brain index`'s recheck judges a gap as a search does: with the runtime, a new
/// note that only shares the question's words leaves the gap open, and one that answers fills it.
#[test]
fn with_the_runtime_a_gap_is_filled_only_by_an_answer() {
    let ort = std::env::var_os("ORT_DYLIB_PATH")
        .expect("set ORT_DYLIB_PATH (scripts/fetch-ort.sh); CI does");
    let h = Home::new();
    h.init();
    let run = |args: &[&str]| {
        let out = h
            .brain()
            .env("ORT_DYLIB_PATH", &ort)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    h.put(
        "recipe.md",
        "French toast: bread soaked in eggs and milk, fried in butter, a breakfast in France.\n",
    );
    run(&["index"]);
    run(&["search", "what is the capital of France"]);
    assert_eq!(open_gaps(&h).len(), 1);
    h.put(
        "packing.md",
        "Packing for France: passport, plug adapter, a phrasebook for the capital's cafes.\n",
    );
    let out = run(&["index"]);
    assert!(!out.contains("gap filled"), "{out}");
    assert_eq!(open_gaps(&h).len(), 1);
    h.put(
        "answer.md",
        "Paris is the capital and largest city of France.\n",
    );
    let out = run(&["index"]);
    assert!(out.contains("1 gap filled"), "{out}");
}
