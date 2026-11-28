//! Capture, correct and due through the CLI; the MCP server shares the same functions.

use std::collections::HashSet;
use std::fs;

use super::Home;

fn ids_in_inbox(h: &Home) -> Vec<String> {
    h.inbox()
        .iter()
        .map(|n| {
            let text = fs::read_to_string(h.vault.path().join("0-inbox").join(n)).unwrap();
            text.lines()
                .find_map(|l| l.strip_prefix("id: "))
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn two_captures_with_one_title_both_land() {
    let h = Home::new();
    h.init();
    std::thread::scope(|s| {
        for n in 0..16 {
            let h = &h;
            s.spawn(move || h.run(&["add", &format!("capture {n}"), "--title", "Same"]));
        }
    });
    assert_eq!(h.inbox().len(), 16, "{:?}", h.inbox());
    let ids: HashSet<String> = ids_in_inbox(&h).into_iter().collect();
    assert_eq!(ids.len(), 16);
    let bodies: HashSet<String> = h
        .inbox()
        .iter()
        .map(|n| fs::read_to_string(h.vault.path().join("0-inbox").join(n)).unwrap())
        .map(|t| t.lines().last().unwrap().to_string())
        .collect();
    assert_eq!(bodies.len(), 16, "no capture was written over another");
}

#[test]
fn add_output_is_unchanged() {
    let h = Home::new();
    h.init();
    assert_eq!(
        h.run(&["add", "the gate code", "--title", "Gate"]),
        "0-inbox/gate.md\n"
    );
    assert_eq!(
        h.run(&["add", "another", "--title", "Gate"]),
        "0-inbox/gate-1.md\n"
    );
}

#[test]
fn correct_writes_a_rule_note_that_names_its_target() {
    let h = Home::new();
    h.init();
    let target =
        "---\nid: 0195f2e0-0000-7000-8000-0000000000c1\ntitle: Bins\n---\nbins go out on Monday\n";
    h.put("3-permanent/bins.md", target);
    h.run(&["index"]);
    let rel = h.run(&[
        "--now",
        "2026-11-25T03:00:00Z",
        "correct",
        "0195f2e0-0000-7000-8000-0000000000c1",
        "bins go out on Tuesday since November",
    ]);
    let rel = rel.trim();
    assert!(rel.starts_with("0-inbox/"), "{rel}");
    let note = fs::read_to_string(h.vault.path().join(rel)).unwrap();
    for line in [
        "kind: rule",
        "title: \"Correction: Bins\"",
        "corrects: 0195f2e0-0000-7000-8000-0000000000c1",
        "owner: robin",
    ] {
        assert!(note.contains(line), "{line} missing:\n{note}");
    }
    assert!(
        note.ends_with("bins go out on Tuesday since November\n\nCorrects [[bins]].\n"),
        "{note}"
    );
    assert_eq!(
        fs::read_to_string(h.vault.path().join("3-permanent/bins.md")).unwrap(),
        target
    );
    let by_path = h.run(&["correct", "3-permanent/bins.md", "and Thursdays"]);
    assert!(by_path.starts_with("0-inbox/"));
}

#[test]
fn correct_refuses_an_unknown_target() {
    let h = Home::new();
    h.init();
    h.put(".obsidian/workspace.md", "editor state\n");
    h.put("_archive/old.md", "archived\n");
    h.put("a.md", "a note\n");
    h.run(&["index"]);
    for target in [
        "0195f2e0-0000-7000-8000-0000000000ff",
        "nothing/here.md",
        ".obsidian/workspace.md",
        "_archive/old.md",
    ] {
        let out = h.brain().args(["correct", target, "x"]).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{target}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(&format!("no note is {target}")),
            "{target}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert!(h.inbox().is_empty());
}

#[test]
fn due_prints_mine_oldest_first() {
    let h = Home::new();
    h.init();
    let note = |owner: &str, review: &str, title: &str| {
        format!("---\ntitle: {title}\nowner: {owner}\nreview: {review}\n---\nbody\n")
    };
    h.put("b.md", &note("robin", "2026-11-30", "Second"));
    h.put("a.md", &note("robin", "2026-11-20", "First"));
    h.put("c.md", &note("dana", "2026-11-20", "Hers"));
    h.put("d.md", &note("robin", "2027-03-01", "Later"));
    h.run(&["index"]);
    let now = ["--now", "2026-11-25T03:00:00Z"];
    let out = h.run(&[now[0], now[1], "due"]);
    assert_eq!(
        out, "2026-11-20  a.md  First\n2026-11-30  b.md  Second\n",
        "within 14 days by default"
    );
    let json = h.json(&[now[0], now[1], "due", "--within", "0", "--json"])["due"].clone();
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["path"], "a.md");
    let empty = Home::new();
    empty.init();
    assert_eq!(empty.run(&["due"]), "nothing of yours is due\n");
}

#[test]
fn a_quoted_title_is_searched_as_written() {
    let h = Home::new();
    h.init();
    h.run(&["add", "the zebra crossing", "--title", "say \"hi\": now"]);
    h.run(&["index"]);
    let v = h.json(&["search", "--json", "zebra"]);
    assert_eq!(v["hits"][0]["title"], "say \"hi\": now", "{v}");
}
