//! An index belongs to one vault: nothing from another vault's index is ever shown or counted.

use std::fs;
use std::path::Path;

use serde_json::json;

use super::Home;
use super::mcp::Mcp;

/// A brain with an indexed note about herons, pointed at a new empty vault before any reindex.
fn switched() -> (Home, tempfile::TempDir) {
    let h = Home::new();
    h.init();
    h.put(
        "old.md",
        "---\nowner: robin\nreview: 2020-01-01\n---\nthe heron price\n",
    );
    h.run(&["index"]);
    let next = tempfile::tempdir().unwrap();
    init_at(&h, next.path());
    (h, next)
}

fn init_at(h: &Home, vault: &Path) {
    h.run(&["init", "--me", "robin", "--vault", vault.to_str().unwrap()]);
}

fn refusal(h: &Home, args: &[&str]) -> String {
    let out = h.brain().args(args).output().unwrap();
    assert!(!out.status.success(), "{args:?}: {out:?}");
    assert!(out.stdout.is_empty(), "{args:?}: {out:?}");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn after_init_over_a_new_vault_nothing_old_is_shown() {
    let (h, next) = switched();
    let new = next.path().canonicalize().unwrap();
    let old = h.vault.path().canonicalize().unwrap();
    // By path a note is read from the vault as configured; by id it is looked up in the index.
    let written = fs::read_to_string(old.join("old.md")).unwrap();
    let id = written
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap();
    for args in [
        &["search", "heron"][..],
        &["due"],
        &["show", id],
        &["correct", "old.md", "wrong"],
    ] {
        let said = refusal(&h, args);
        assert!(said.contains("still holds"), "{args:?}: {said}");
        assert!(said.contains(&old.display().to_string()), "{said}");
        assert!(said.contains(&new.display().to_string()), "{said}");
        assert!(!said.contains("price"), "{said}");
    }
    let status = h.json(&["status", "--json"]);
    assert_eq!(status["notes"], json!(null), "{status}");
    assert!(
        status["index"].as_str().unwrap().contains("still holds"),
        "{status}"
    );
    assert!(h.run(&["status"]).contains("still holds"));
}

#[test]
fn a_reindex_makes_the_index_the_new_vaults() {
    let (h, next) = switched();
    fs::write(next.path().join("new.md"), "the egret price\n").unwrap();
    h.run(&["index"]);
    assert_eq!(h.json(&["status", "--json"])["notes"], 1);
    let found = h.run(&["search", "price"]);
    assert!(
        found.contains("new.md") && !found.contains("old.md"),
        "{found}"
    );
}

/// A file with the same path, size and time as one in the old vault is not read again, and must
/// not be taken for it.
#[test]
fn a_file_that_looks_unchanged_keeps_nothing_of_the_old_vault() {
    let (h, next) = switched();
    let old = h.vault.path().join("old.md");
    let new = next.path().join("old.md");
    let text = fs::read_to_string(&old).unwrap().replace("heron", "egret");
    fs::write(&new, text).unwrap();
    let modified = fs::metadata(&old).unwrap().modified().unwrap();
    fs::File::options()
        .write(true)
        .open(&new)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    h.run(&["index"]);
    let found = h.run(&["search", "price"]);
    assert!(
        found.contains("egret") && !found.contains("heron"),
        "{found}"
    );
}

#[test]
fn a_session_start_shows_nothing_due_from_another_vault() {
    let (h, _next) = switched();
    let out = h
        .brain()
        .args(["hook", "session-start"])
        .write_stdin("{}")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "{out:?}");
}

#[test]
fn the_mcp_search_refuses_another_vaults_index() {
    let (h, _next) = switched();
    let mut mcp = Mcp::start(&h);
    for (tool, args) in [
        ("search_my_brain", json!({"query": "heron"})),
        ("search_company", json!({"query": "heron"})),
        ("due", json!({})),
    ] {
        let (text, error) = mcp.text(tool, args);
        assert!(error, "{tool}: {text}");
        assert!(text.contains("still holds"), "{tool}: {text}");
        assert!(!text.contains("price"), "{tool}: {text}");
    }
}

/// An AI app stays open across a `brain init`, and must write to the new vault.
#[test]
fn a_long_lived_mcp_follows_the_new_vault() {
    let h = Home::new();
    h.init();
    h.put("old.md", "the heron price\n");
    h.run(&["index"]);
    let mut mcp = Mcp::start(&h);
    let next = tempfile::tempdir().unwrap();
    init_at(&h, next.path());
    let (text, error) = mcp.text("capture", json!({"text": "a new thought"}));
    assert!(!error, "{text}");
    assert_eq!(
        fs::read_dir(next.path().join("0-inbox")).unwrap().count(),
        1
    );
    assert_eq!(h.inbox(), Vec::<String>::new());
    h.run(&["index"]);
    let (text, error) = mcp.text("search_my_brain", json!({"query": "thought"}));
    assert!(!error && text.contains("new thought"), "{text}");
}
