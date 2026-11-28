use std::path::Path;

use brain_note::gate::{GateFacts, Route, facts, route};
use brain_note::{Note, Place, Split};

fn facts_of(raw: &str) -> GateFacts {
    let note = Note::parse(Path::new("0-inbox/n.md"), raw, Place::Inbox);
    facts(&note, &Split::parse(raw))
}

fn clean(links: usize) -> GateFacts {
    GateFacts {
        markers: Vec::new(),
        wikilinks: links,
        has_source: true,
        body_lines: 5,
        missing_keys: Vec::new(),
    }
}

#[test]
fn a_marker_outside_code_routes_to_fleeting() {
    let f =
        facts_of("---\nsource: a book\n---\nThis claim is [Unverified] and [citation needed].\n");
    assert_eq!(f.markers, vec!["[unverified]", "[citation needed]"]);
    assert!(f.has_source);
    assert_eq!(route(&f, Some(6), false), Route::Fleeting);
    assert_eq!(route(&f, None, false), Route::Fleeting);
}

#[test]
fn a_marker_inside_a_fence_does_not_count() {
    let raw =
        "body line\n```\nlet s = \"[unresolved]\";\n```\n~~~md\n[not in source]\n~~~\nafter\n";
    let f = facts_of(raw);
    assert_eq!(f.markers, Vec::<String>::new());
    assert_eq!(f.body_lines, 8);
    assert!(!f.has_source);
    assert_eq!(f.missing_keys, vec!["id", "date", "owner", "review"]);
}

#[test]
fn marks_that_open_no_block_hide_nothing() {
    let raw = "```x``` is code [unresolved]\n\n    ```\n\nthen [unverified]\n";
    assert_eq!(facts_of(raw).markers, vec!["[unresolved]", "[unverified]"]);
}

#[test]
fn unjudged_never_routes_to_permanent() {
    assert_eq!(route(&clean(12), None, true), Route::Inbox);
    assert_eq!(route(&clean(0), None, false), Route::Inbox);
}

#[test]
fn six_judged_routes_to_permanent() {
    assert_eq!(route(&clean(2), Some(6), false), Route::Permanent);
    for n in 3..=5 {
        assert_eq!(route(&clean(2), Some(n), false), Route::Fleeting, "{n}");
    }
    for n in 0..=2 {
        assert_eq!(route(&clean(2), Some(n), false), Route::Inbox, "{n}");
    }
}

#[test]
fn a_synthesis_hub_routes_to_maps() {
    assert_eq!(route(&clean(10), Some(6), true), Route::Maps);
    assert_eq!(route(&clean(9), Some(6), true), Route::Permanent);
    assert_eq!(route(&clean(10), Some(6), false), Route::Permanent);
    assert_eq!(
        facts_of("[[a]] [[b]] [[a|again]] and [[c#x]]\n").wikilinks,
        3
    );
}
