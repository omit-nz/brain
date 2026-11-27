//! Archiving is undone when a source returns: the same file comes back under the same id.

use serde_json::Value;

use super::*;

const LEDGER: &str = "data/import-ledger.json";
const ARCHIVE: &str = "data/follow-archive.json";

fn archive_path(h: &Home, name: &str) -> PathBuf {
    h.vault
        .path()
        .join("_archive/working-memory/-proj")
        .join(format!("{name}.md"))
}

fn id_of(text: &str) -> String {
    let line = text.lines().find(|l| l.starts_with("id:")).unwrap();
    line[3..].trim().trim_matches('"').to_string()
}

fn id(h: &Home, name: &str) -> String {
    id_of(&copy(h, name).unwrap())
}

fn records(h: &Home) -> Vec<Value> {
    fs::read_to_string(h.home.path().join(ARCHIVE))
        .map_or_else(|_| Vec::new(), |text| serde_json::from_str(&text).unwrap())
}

fn recorded(h: &Home) -> Vec<String> {
    records(h)
        .iter()
        .map(|r| r["key"].as_str().unwrap().to_string())
        .collect()
}

fn ledger(h: &Home) -> Value {
    serde_json::from_str(&fs::read_to_string(h.home.path().join(LEDGER)).unwrap()).unwrap()
}

fn set_ledger(h: &Home, ledger: &Value) {
    fs::write(h.home.path().join(LEDGER), ledger.to_string()).unwrap();
}

/// `one` and `stays` followed, so removing `one` never leaves the folder empty.
fn two() -> (Home, Projects) {
    following(&["one", "stays"])
}

/// Removes `one`'s source and looks twice, which archives its copy.
fn archive_one(h: &Home, p: &Projects) {
    p.remove("one");
    pass(h);
    let said = pass(h);
    assert!(said.contains("1 archived"), "{said}");
}

/// `one` archived, with its source's bytes and its copy's, to bring either back.
fn archived_one() -> (Home, Projects, Vec<u8>, String) {
    let (h, p) = two();
    let source = fs::read(p.file("one")).unwrap();
    let copied = copy(&h, "one").unwrap();
    archive_one(&h, &p);
    (h, p, source, copied)
}

fn traced(h: &Home, id: &str) -> usize {
    let store = omit_agent_trace::Store::open(&h.home.path().join("data/traces.db")).unwrap();
    let source = format!("note:{id}");
    store
        .runs()
        .unwrap()
        .iter()
        .filter(|r| {
            let spans = store.spans(&r.id).unwrap();
            spans.iter().any(|s| s.sources.contains(&source))
        })
        .count()
}

#[test]
fn a_returned_source_brings_back_the_same_file_and_id() {
    let (h, p, source, copied) = archived_one();
    assert_eq!(copy(&h, "one"), None);
    fs::write(p.file("one"), source).unwrap();
    let said = pass(&h);
    assert!(said.contains("1 restored"), "{said}");
    assert!(said.contains("0 copied, 0 updated"), "{said}");
    assert_eq!(copy(&h, "one"), Some(copied));
    assert!(!archive_path(&h, "one").exists(), "moved back, not copied");
    pass(&h);
    assert!(records(&h).is_empty(), "{:?}", records(&h));
}

#[test]
fn a_restore_only_pass_is_searchable() {
    let (h, p, source, _) = archived_one();
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    let found = h.json(&["search", "--json", "one"]);
    let paths: Vec<&str> = found["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"working-memory/-proj/one.md"), "{paths:?}");
}

/// A reset folder, most of it back first and the rest later.
#[test]
fn a_folder_reset_and_restored_in_halves_leaves_the_vault_as_it_was() {
    let names: Vec<String> = (0..24).map(|i| format!("m{i}")).collect();
    let mut all: Vec<&str> = names.iter().map(String::as_str).collect();
    all.push("stays");
    let (h, p) = following(&all);
    let sources: Vec<Vec<u8>> = names.iter().map(|n| fs::read(p.file(n)).unwrap()).collect();
    let copies: Vec<String> = names.iter().map(|n| copy(&h, n).unwrap()).collect();
    for name in &names {
        p.remove(name);
    }
    pass(&h);
    let said = pass(&h);
    assert!(
        said.contains("24 archived"),
        "any number is archived: {said}"
    );
    for (name, source) in names.iter().zip(&sources).take(12) {
        fs::write(p.file(name), source).unwrap();
    }
    pass(&h);
    pass(&h);
    pass(&h);
    assert!(names[..12].iter().all(|n| copy(&h, n).is_some()));
    assert!(names[12..].iter().all(|n| archive_path(&h, n).exists()));
    assert_eq!(records(&h).len(), 12);
    for (name, source) in names.iter().zip(&sources).skip(12) {
        fs::write(p.file(name), source).unwrap();
    }
    pass(&h);
    let back: Vec<String> = names.iter().map(|n| copy(&h, n).unwrap()).collect();
    assert_eq!(back, copies);
}

#[test]
fn a_source_that_returns_changed_keeps_the_old_words_in_the_archive() {
    let (h, p, _, copied) = archived_one();
    p.write("one", "one second");
    pass(&h);
    let now = copy(&h, "one").unwrap();
    assert!(now.contains("one second"), "{now}");
    assert_eq!(id_of(&now), id_of(&copied));
    assert_eq!(fs::read_to_string(archive_path(&h, "one")).unwrap(), copied);
    pass(&h);
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    // Gone again, the new copy goes beside the old one, never over it.
    archive_one(&h, &p);
    assert_eq!(fs::read_to_string(archive_path(&h, "one")).unwrap(), copied);
    assert!(archive_path(&h, "one-1").exists());
}

#[test]
fn an_edited_archived_file_comes_back_and_survives_its_source_changing() {
    let (h, p, source, copied) = archived_one();
    let mine = format!("{copied}\nmy own words\n");
    fs::write(archive_path(&h, "one"), &mine).unwrap();
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(mine.clone()));
    pass(&h);
    p.write("one", "one second");
    let said = pass(&h);
    assert_eq!(copy(&h, "one"), Some(mine));
    assert!(said.contains("kept as it is"), "{said}");
}

#[test]
fn an_archived_file_the_person_deleted_stays_removed() {
    let (h, p, source, _) = archived_one();
    fs::remove_file(archive_path(&h, "one")).unwrap();
    fs::write(p.file("one"), source).unwrap();
    for _ in 0..3 {
        let said = pass(&h);
        assert_eq!(copy(&h, "one"), None, "{said}");
    }
    assert!(records(&h).is_empty(), "{:?}", records(&h));
}

#[test]
fn a_removed_copy_is_not_copied_again_when_its_source_goes_and_returns() {
    let (h, p) = two();
    let source = fs::read(p.file("one")).unwrap();
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    p.remove("one");
    pass(&h);
    pass(&h);
    pass(&h);
    fs::write(p.file("one"), source).unwrap();
    let said = pass(&h);
    assert_eq!(copy(&h, "one"), None, "{said}");
}

#[test]
fn a_removed_copy_whose_source_changes_is_not_written_again() {
    let (h, p) = two();
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    p.write("one", "one second");
    let said = pass(&h);
    assert_eq!(copy(&h, "one"), None, "{said}");
}

#[test]
fn a_taken_copy_path_waits_and_then_restores() {
    let (h, p, source, copied) = archived_one();
    h.put("working-memory/-proj/one.md", "mine\n");
    fs::write(p.file("one"), source).unwrap();
    let said = pass(&h);
    assert!(said.contains("kept as it is"), "{said}");
    pass(&h);
    assert_eq!(copy(&h, "one").as_deref(), Some("mine\n"));
    assert!(archive_path(&h, "one").exists());
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(copied));
}

#[test]
fn an_edited_copy_whose_source_returns_is_left_alone() {
    let (h, p) = two();
    let source = fs::read(p.file("one")).unwrap();
    h.put("working-memory/-proj/one.md", "my own words\n");
    p.remove("one");
    pass(&h);
    pass(&h);
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one").as_deref(), Some("my own words\n"));
    assert!(!archive_path(&h, "one").exists());
}

#[test]
fn removed_copies_alone_cause_no_refusal() {
    let (h, p) = following(&["one"]);
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    p.remove("one");
    let said = pass(&h);
    assert!(said.contains("0 copied"), "{said}");
}

#[test]
fn archived_copies_alone_cause_no_refusal() {
    let (h, p) = two();
    archive_one(&h, &p);
    h.run(&["memory", "unfollow"]);
    let empty = Projects::new();
    let said = follow(&h, &empty);
    assert!(said.contains("0 archived"), "{said}");
}

#[test]
fn an_unreadable_follow_archive_changes_nothing() {
    let (h, p, source, _) = archived_one();
    fs::write(h.home.path().join(ARCHIVE), "not json").unwrap();
    fs::write(p.file("one"), source).unwrap();
    let out = h.brain().args(["memory", "run"]).output().unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(copy(&h, "one"), None);
    assert!(archive_path(&h, "one").exists());
}

/// A pass killed after its moves and before it saved the ledger.
#[test]
fn an_archive_cut_short_after_the_move_is_finished() {
    let (h, p) = two();
    let source = fs::read(p.file("one")).unwrap();
    let copied = copy(&h, "one").unwrap();
    p.remove("one");
    pass(&h);
    let before = ledger(&h);
    pass(&h);
    set_ledger(&h, &before);
    let said = pass(&h);
    assert!(said.contains("0 archived"), "{said}");
    assert!(ledger(&h).get("memory/-proj/one.md").is_none());
    assert_eq!(recorded(&h), ["memory/-proj/one.md"]);
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(copied));
}

/// A pass killed after it recorded the archive and before it moved the file.
#[test]
fn an_archive_cut_short_before_the_move_archives_again() {
    let (h, p) = two();
    p.remove("one");
    pass(&h);
    let before = ledger(&h);
    pass(&h);
    set_ledger(&h, &before);
    fs::rename(archive_path(&h, "one"), copy_path(h.vault.path(), "one")).unwrap();
    let said = pass(&h);
    assert!(said.contains("1 archived"), "{said}");
    assert_eq!(copy(&h, "one"), None);
    assert!(archive_path(&h, "one").exists());
    assert_eq!(records(&h).len(), 1);
}

/// A pass killed after it moved a file back and before it saved the ledger.
#[test]
fn a_restore_cut_short_is_finished() {
    let (h, p, source, copied) = archived_one();
    fs::write(p.file("one"), source).unwrap();
    fs::rename(archive_path(&h, "one"), copy_path(h.vault.path(), "one")).unwrap();
    let said = pass(&h);
    assert!(said.contains("0 copied"), "{said}");
    assert!(!said.contains("kept as it is"), "{said}");
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    p.write("one", "one second");
    pass(&h);
    let now = copy(&h, "one").unwrap();
    assert!(now.contains("one second"), "{now}");
    assert_eq!(id_of(&now), id_of(&copied));
}

/// A reconcile killed after it saved the ledger and before it saved the archive records.
#[test]
fn a_reconcile_cut_short_between_its_saves_is_finished() {
    let (h, p, source, copied) = archived_one();
    let before = fs::read(h.home.path().join(ARCHIVE)).unwrap();
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    pass(&h);
    fs::write(h.home.path().join(ARCHIVE), before).unwrap();
    pass(&h);
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    assert_eq!(copy(&h, "one"), Some(copied));
}

/// The ledger as a reconcile leaves it when killed before it saves the records: the record's own
/// entry under its key, and the record still there.
fn ledger_took_the_record(h: &Home) {
    let mut entry = records(h)[0].clone();
    let fields = entry.as_object_mut().unwrap();
    let key = fields.remove("key").unwrap();
    fields.remove("path");
    fields.remove("gone");
    let mut ledger = ledger(h);
    ledger[key.as_str().unwrap()] = entry;
    set_ledger(h, &ledger);
}

/// Row 4 half done: the file moved back by hand, the ledger saved, the record not yet dropped.
#[test]
fn a_moved_back_file_half_reconciled_keeps_its_words_and_id() {
    let (h, p, source, copied) = archived_one();
    fs::rename(archive_path(&h, "one"), copy_path(h.vault.path(), "one")).unwrap();
    ledger_took_the_record(&h);
    fs::write(p.file("one"), source).unwrap();
    let said = pass(&h);
    assert!(said.contains("0 copied, 0 updated"), "{said}");
    assert!(!said.contains("kept as it is"), "{said}");
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    assert_eq!(copy(&h, "one"), Some(copied));
}

/// Row 7 half done: the archived file deleted, the ledger saved, the record not yet dropped.
#[test]
fn a_deleted_archived_file_half_reconciled_stays_removed() {
    let (h, p, source, _) = archived_one();
    fs::remove_file(archive_path(&h, "one")).unwrap();
    pass(&h);
    ledger_took_the_record(&h);
    fs::write(p.file("one"), source).unwrap();
    let said = pass(&h);
    assert_eq!(copy(&h, "one"), None, "{said}");
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    let said = pass(&h);
    assert_eq!(copy(&h, "one"), None, "{said}");
}

/// A changed source's fresh copy written, and the pass killed before the ledger was saved.
#[test]
fn a_changed_copy_cut_short_before_the_ledger_save_is_not_taken_for_the_old_one() {
    let (h, p, _, copied) = archived_one();
    p.write("one", "one second");
    let before = ledger(&h);
    pass(&h);
    let fresh = copy(&h, "one").unwrap();
    set_ledger(&h, &before);
    let said = pass(&h);
    assert_eq!(copy(&h, "one"), Some(fresh));
    assert_eq!(fs::read_to_string(archive_path(&h, "one")).unwrap(), copied);
    assert_eq!(records(&h).len(), 1, "{said}");
    // The person removes it, and it is written again under the same id.
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    pass(&h);
    let again = copy(&h, "one").unwrap();
    assert!(again.contains("one second"), "{again}");
    assert_eq!(id_of(&again), id_of(&copied));
}

#[test]
fn an_import_copies_an_archived_memory_afresh() {
    let (h, p, source, copied) = archived_one();
    fs::write(p.file("one"), source).unwrap();
    import(&h, p.path());
    assert_ne!(id(&h, "one"), id_of(&copied));
    pass(&h);
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    assert_eq!(fs::read_to_string(archive_path(&h, "one")).unwrap(), copied);
    assert!(
        traced(&h, &id_of(&copied)) >= 1,
        "its archived file is still there"
    );
    fs::remove_file(archive_path(&h, "one")).unwrap();
    pass(&h);
    pass(&h);
    h.run(&["index"]);
    assert!(
        traced(&h, &id_of(&copied)) >= 1,
        "untracked: purges nothing"
    );
}

#[test]
fn a_file_found_again_starts_the_two_looks_over() {
    let (h, p, source, copied) = archived_one();
    let aside = h.vault.path().join("_archive/aside.md");
    let move_aside = || fs::rename(archive_path(&h, "one"), &aside).unwrap();
    let move_back = || fs::rename(&aside, archive_path(&h, "one")).unwrap();
    move_aside();
    pass(&h);
    move_back();
    pass(&h);
    move_aside();
    pass(&h);
    move_back();
    assert_eq!(records(&h).len(), 1);
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(copied));
}

#[test]
fn an_archived_memory_keeps_its_traces() {
    let (h, p) = two();
    let one = id(&h, "one");
    assert_eq!(traced(&h, &one), 1);
    archive_one(&h, &p);
    h.run(&["index"]);
    assert!(traced(&h, &one) >= 1);
}

#[test]
fn deleting_the_archived_file_purges_its_traces_on_the_second_look() {
    let (h, p) = two();
    let one = id(&h, "one");
    archive_one(&h, &p);
    fs::remove_file(archive_path(&h, "one")).unwrap();
    pass(&h);
    assert!(traced(&h, &one) >= 1, "one look purges nothing");
    pass(&h);
    assert_eq!(traced(&h, &one), 0);
}

#[test]
fn an_unreadable_follow_archive_purges_nothing() {
    let (h, p) = two();
    let one = id(&h, "one");
    archive_one(&h, &p);
    // Put the index back to holding the note, then take it away with the records unreadable.
    fs::rename(archive_path(&h, "one"), copy_path(h.vault.path(), "one")).unwrap();
    h.run(&["index"]);
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    fs::write(h.home.path().join(ARCHIVE), "not json").unwrap();
    h.run(&["index"]);
    assert!(traced(&h, &one) >= 1);
}

#[test]
fn an_archived_memory_is_in_no_search_due_or_brief() {
    let (h, p) = two();
    let path = copy_path(h.vault.path(), "one");
    // Due now, which brain's own copies are not for weeks.
    let text = fs::read_to_string(&path).unwrap();
    let review = text.lines().find(|l| l.starts_with("review:")).unwrap();
    fs::write(&path, text.replace(review, "review: 2020-01-01")).unwrap();
    h.run(&["index"]);
    let shown = || {
        let search = h.json(&["search", "--json", "first"]).to_string();
        let due = h.run(&["due"]);
        let brief = h
            .brain()
            .args(["hook", "session-start"])
            .write_stdin(r#"{"session_id":"s","cwd":"/","transcript_path":"/t/s.jsonl"}"#)
            .output()
            .unwrap();
        let brief = String::from_utf8(brief.stdout).unwrap();
        [search, due, brief].map(|said| said.contains("-proj/one.md") || said.contains("\"one\""))
    };
    assert_eq!(shown(), [true; 3], "search, due, brief, before");
    // An edited copy is not archived, so the source goes with the ledger agreeing with it.
    let mut ledger = ledger(&h);
    ledger["memory/-proj/one.md"]["dst_hash"] = sha256(&fs::read(&path).unwrap()).into();
    set_ledger(&h, &ledger);
    archive_one(&h, &p);
    assert_eq!(shown(), [false; 3], "search, due, brief, after");
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn status_counts_what_is_archived() {
    let (h, p) = two();
    assert!(!h.run(&["status"]).contains("out of search"));
    assert_eq!(h.json(&["status", "--json"])["memories_archived"], 0);
    archive_one(&h, &p);
    let said = h.run(&["status"]);
    assert!(
        said.contains("1 followed memory is gone from Claude Code and out of search"),
        "{said}"
    );
    assert_eq!(h.json(&["status", "--json"])["memories_archived"], 1);
    fs::remove_file(archive_path(&h, "one")).unwrap();
    assert_eq!(h.json(&["status", "--json"])["memories_archived"], 0);
    fs::write(h.home.path().join(ARCHIVE), "not json").unwrap();
    assert!(h.json(&["status", "--json"])["memories_archived"].is_null());
    assert!(h.run(&["status"]).contains("cannot be read"));
}

#[test]
fn another_followed_folder_restores_nothing_and_takes_no_id() {
    let (h, p) = two();
    let copied = copy(&h, "one").unwrap();
    archive_one(&h, &p);
    h.run(&["memory", "unfollow"]);
    let other = Projects::new();
    other.write("one", "one first");
    follow(&h, &other);
    assert_ne!(id(&h, "one"), id_of(&copied));
    assert_eq!(fs::read_to_string(archive_path(&h, "one")).unwrap(), copied);
    pass(&h);
    assert!(
        traced(&h, &id_of(&copied)) >= 1,
        "its archived file is still there"
    );
    fs::remove_file(archive_path(&h, "one")).unwrap();
    pass(&h);
    pass(&h);
    h.run(&["index"]);
    assert!(
        traced(&h, &id_of(&copied)) >= 1,
        "untracked: purges nothing"
    );
}

#[test]
fn another_vault_restores_nothing_and_switching_back_restores() {
    let (h, p, source, copied) = archived_one();
    let next = tempfile::tempdir().unwrap();
    let init = |vault: &Path| {
        h.run(&["init", "--me", "robin", "--vault", vault.to_str().unwrap()]);
    };
    init(next.path());
    for _ in 0..3 {
        pass(&h);
    }
    assert_eq!(records(&h).len(), 1, "nothing ends in another vault");
    assert!(archive_path(&h, "one").exists());
    init(h.vault.path());
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(copied));
}

/// brain 0.2.4 saves each record as its id and two hashes, and never names the archive records.
#[test]
fn a_ledger_trimmed_by_an_older_release_still_restores() {
    let (h, p, source, copied) = archived_one();
    let mut trimmed = ledger(&h);
    for entry in trimmed.as_object_mut().unwrap().values_mut() {
        let entry = entry.as_object_mut().unwrap();
        entry.retain(|k, _| ["id", "src_hash", "dst_hash"].contains(&k.as_str()));
    }
    set_ledger(&h, &trimmed);
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(copied));
}

#[test]
fn an_archived_file_rewritten_without_its_id_reads_as_deleted() {
    let (h, p, source, _) = archived_one();
    fs::write(archive_path(&h, "one"), "rewritten\n").unwrap();
    pass(&h);
    pass(&h);
    fs::write(p.file("one"), source).unwrap();
    let said = pass(&h);
    assert_eq!(copy(&h, "one"), None, "{said}");
    assert_eq!(
        fs::read_to_string(archive_path(&h, "one")).unwrap(),
        "rewritten\n"
    );
}

#[test]
fn an_archived_file_moved_back_while_its_source_is_gone_is_archived_again() {
    let (h, p) = two();
    archive_one(&h, &p);
    fs::rename(archive_path(&h, "one"), copy_path(h.vault.path(), "one")).unwrap();
    pass(&h);
    pass(&h);
    assert_eq!(copy(&h, "one"), None);
    assert!(archive_path(&h, "one").exists());
}

/// A project folder moved with nothing to say it was moved.
#[test]
fn a_folder_moved_with_no_link_is_copied_anew() {
    let (h, p) = two();
    let copied = copy(&h, "one").unwrap();
    fs::remove_file(copy_path(h.vault.path(), "stays")).unwrap();
    let (old, new) = (p.path().join("-proj"), p.path().join("-new"));
    fs::rename(&old, &new).unwrap();
    pass(&h);
    pass(&h);
    let moved = h.vault.path().join("working-memory/-new/one.md");
    assert_ne!(id_of(&fs::read_to_string(&moved).unwrap()), id_of(&copied));
    assert_eq!(copy(&h, "one"), None);
    let back = h.vault.path().join("working-memory/-new/stays.md");
    assert!(back.exists(), "a removed copy is copied again");
    fs::rename(&new, &old).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(copied));
}

/// Renames the vault folder and sets brain up over it where it now is.
fn rename_vault(h: &Home) -> (tempfile::TempDir, PathBuf) {
    let aside = tempfile::tempdir().unwrap();
    let renamed = aside.path().join("renamed");
    fs::rename(h.vault.path(), &renamed).unwrap();
    h.brain()
        .current_dir(h.home.path())
        .args([
            "init",
            "--me",
            "robin",
            "--vault",
            renamed.to_str().unwrap(),
        ])
        .assert()
        .success();
    (aside, renamed)
}

#[test]
fn a_renamed_vault_folder_stops_following_its_copies() {
    let (h, p) = following(&["one", "two", "three"]);
    let archived = (fs::read(p.file("three")).unwrap(), id(&h, "three"));
    p.remove("three");
    pass(&h);
    pass(&h);
    fs::remove_file(copy_path(h.vault.path(), "two")).unwrap();
    let (_aside, renamed) = rename_vault(&h);
    p.write("one", "one second");
    let run = || {
        h.brain()
            .current_dir(h.home.path())
            .args(["memory", "run"])
            .assert()
            .success()
    };
    fs::write(p.file("three"), archived.0).unwrap();
    run();
    let three = fs::read_to_string(copy_path(&renamed, "three")).unwrap();
    assert_ne!(id_of(&three), archived.1, "not brought back");
    let text = fs::read_to_string(copy_path(&renamed, "one")).unwrap();
    assert!(text.contains("one first"), "no longer followed: {text}");
    assert!(
        copy_path(&renamed, "two").exists(),
        "a removed copy is back"
    );
}

#[test]
fn a_pass_killed_before_the_ledger_save_leaves_its_copies_as_the_persons() {
    let (h, p) = two();
    let before = ledger(&h);
    p.write("one", "one second");
    pass(&h);
    set_ledger(&h, &before);
    p.write("one", "one third");
    let said = pass(&h);
    assert!(said.contains("kept as it is"), "{said}");
    assert!(copy(&h, "one").unwrap().contains("one second"));
}

#[test]
fn a_source_that_comes_and_goes_moves_its_file_each_time() {
    let (h, p) = two();
    let source = fs::read(p.file("one")).unwrap();
    for _ in 0..2 {
        archive_one(&h, &p);
        assert!(archive_path(&h, "one").exists());
        fs::write(p.file("one"), &source).unwrap();
        let said = pass(&h);
        assert!(said.contains("1 restored"), "{said}");
    }
}

#[test]
fn a_vault_without_its_archive_folder_reads_as_the_files_deleted() {
    let (h, p) = two();
    let one = id(&h, "one");
    let source = fs::read(p.file("one")).unwrap();
    archive_one(&h, &p);
    fs::remove_dir_all(h.vault.path().join("_archive")).unwrap();
    pass(&h);
    pass(&h);
    assert_eq!(traced(&h, &one), 0);
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), None);
}

/// A spare of a restored file put back where it was archived: the live copy stays followed.
#[test]
fn a_spare_at_the_archived_path_does_not_unrecord_the_live_copy() {
    let (h, p, source, copied) = archived_one();
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    fs::write(archive_path(&h, "one"), &copied).unwrap();
    let said = pass(&h);
    assert!(!said.contains("kept as it is"), "{said}");
    assert_eq!(h.json(&["status", "--json"])["memories_archived"], 0);
    p.write("one", "one second");
    pass(&h);
    assert!(copy(&h, "one").unwrap().contains("one second"));
}

/// The index gives a restored copy a new id when another note holds its own.
#[test]
fn a_restored_copy_the_index_re_ids_is_still_archived_when_it_goes() {
    let (h, p, source, copied) = archived_one();
    h.put("0-inbox/spare.md", &copied);
    h.run(&["index"]);
    fs::write(p.file("one"), source).unwrap();
    pass(&h);
    pass(&h);
    assert_ne!(id(&h, "one"), id_of(&copied), "the index gave it a new id");
    assert!(
        traced(&h, &id_of(&copied)) >= 1,
        "the other note's traces stay"
    );
    archive_one(&h, &p);
}

#[test]
fn an_archived_file_moved_into_the_notes_keeps_its_traces() {
    let (h, p) = two();
    let one = id(&h, "one");
    archive_one(&h, &p);
    let kept = h.vault.path().join("kept.md");
    fs::rename(archive_path(&h, "one"), &kept).unwrap();
    h.run(&["index"]);
    pass(&h);
    pass(&h);
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    assert!(traced(&h, &one) >= 1);
}

#[cfg(unix)]
#[test]
fn an_archived_file_that_cannot_be_read_is_said_and_left() {
    use std::os::unix::fs::PermissionsExt;
    let (h, p, source, copied) = archived_one();
    let mode = |m| {
        fs::set_permissions(archive_path(&h, "one"), fs::Permissions::from_mode(m)).unwrap();
    };
    mode(0o000);
    fs::write(p.file("one"), source).unwrap();
    for _ in 0..2 {
        let said = pass(&h);
        assert!(said.contains("skipped: memory/-proj/one.md"), "{said}");
        assert_eq!(copy(&h, "one"), None);
    }
    mode(0o644);
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(copied));
}

#[test]
fn a_deleted_archived_file_superseded_before_the_second_look_purges_nothing() {
    let (h, p) = two();
    let one = id(&h, "one");
    archive_one(&h, &p);
    fs::remove_file(archive_path(&h, "one")).unwrap();
    h.run(&["memory", "unfollow"]);
    let other = Projects::new();
    other.write("one", "one first");
    follow(&h, &other);
    pass(&h);
    assert!(records(&h).is_empty(), "{:?}", records(&h));
    assert!(traced(&h, &one) >= 1);
}

/// A changed source's fresh copy shares the archived memory's id, and so its traces.
#[test]
fn removing_a_changed_copy_purges_the_archived_memorys_traces_too() {
    let (h, p, _, copied) = archived_one();
    p.write("one", "one second");
    pass(&h);
    pass(&h);
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    h.run(&["index"]);
    assert_eq!(traced(&h, &id_of(&copied)), 0);
    assert!(archive_path(&h, "one").exists());
}

#[cfg(unix)]
#[test]
fn a_copy_path_that_cannot_be_read_is_said_and_left() {
    use std::os::unix::fs::PermissionsExt;
    let (h, _p, _, _) = archived_one();
    h.put("working-memory/-proj/one.md", "mine\n");
    let path = copy_path(h.vault.path(), "one");
    let mode = |m| fs::set_permissions(&path, fs::Permissions::from_mode(m)).unwrap();
    mode(0o000);
    let said = pass(&h);
    mode(0o644);
    assert!(said.contains("skipped: memory/-proj/one.md"), "{said}");
    assert_eq!(records(&h).len(), 1);
}

/// An archived file that cannot be read can outlast a newer record of the same source.
#[cfg(unix)]
#[test]
fn a_newer_archive_of_a_source_takes_the_older_records_place() {
    use std::os::unix::fs::PermissionsExt;
    let (h, p, _, _) = archived_one();
    p.write("one", "one second");
    pass(&h);
    let mode = |m| {
        fs::set_permissions(archive_path(&h, "one"), fs::Permissions::from_mode(m)).unwrap();
    };
    mode(0o000);
    let newer = (fs::read(p.file("one")).unwrap(), copy(&h, "one").unwrap());
    p.remove("one");
    pass(&h);
    pass(&h);
    assert_eq!(records(&h).len(), 1, "{:?}", records(&h));
    // The older file, readable again and then deleted, ends nothing.
    mode(0o644);
    fs::remove_file(archive_path(&h, "one")).unwrap();
    pass(&h);
    pass(&h);
    fs::write(p.file("one"), newer.0).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), Some(newer.1));
}

#[cfg(unix)]
mod linked {
    use std::os::unix::fs::symlink;

    use super::*;

    fn new_memory(p: &Projects, text: &str) {
        let new = p.path().join("-new/memory");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("one.md"), text).unwrap();
    }

    fn new_copy(h: &Home) -> Option<String> {
        fs::read_to_string(h.vault.path().join("working-memory/-new/one.md")).ok()
    }

    fn link_old_to_new(p: &Projects) {
        fs::remove_dir_all(p.path().join("-proj/memory")).unwrap();
        symlink(p.path().join("-new/memory"), p.path().join("-proj/memory")).unwrap();
    }

    /// The second copy under a linked project is archived, kept recorded, and comes back as
    /// itself when the old name is a folder again.
    #[test]
    fn a_second_copy_comes_back_with_its_id() {
        let (h, p) = following(&["one"]);
        let source = fs::read(p.file("one")).unwrap();
        let copied = copy(&h, "one").unwrap();
        new_memory(&p, "another memory entirely\n");
        pass(&h);
        link_old_to_new(&p);
        for _ in 0..3 {
            pass(&h);
        }
        assert_eq!(copy(&h, "one"), None);
        assert_eq!(recorded(&h), ["memory/-proj/one.md"]);
        fs::remove_file(p.path().join("-proj/memory")).unwrap();
        fs::create_dir_all(p.path().join("-proj/memory")).unwrap();
        fs::write(p.file("one"), source).unwrap();
        pass(&h);
        assert_eq!(copy(&h, "one"), Some(copied));
        assert!(new_copy(&h).unwrap().contains("another memory entirely"));
    }

    /// The new name's copy is archived, then the old name is linked to it: the old name's other
    /// words are never moved onto the new name and written over.
    #[test]
    fn a_move_never_writes_over_a_memory_whose_new_name_was_archived() {
        let (h, p) = following(&["one"]);
        new_memory(&p, "another memory entirely\n");
        fs::write(p.path().join("-new/memory/stays.md"), "stays\n").unwrap();
        pass(&h);
        let new_text = new_copy(&h).unwrap();
        fs::remove_file(p.path().join("-new/memory/one.md")).unwrap();
        pass(&h);
        pass(&h);
        assert_eq!(new_copy(&h), None);
        // The new name's memory returns, changed, as the old name becomes a link to it.
        new_memory(&p, "another memory, changed\n");
        link_old_to_new(&p);
        for _ in 0..3 {
            pass(&h);
        }
        let texts = vault_texts(&h);
        assert!(texts.iter().any(|t| t.contains("one first")), "{texts:?}");
        assert!(texts.contains(&new_text), "{texts:?}");
        assert!(new_copy(&h).unwrap().contains("another memory, changed"));
    }

    #[test]
    fn a_linked_name_never_copied_takes_the_old_copy_and_its_words() {
        let (h, p) = following(&["one"]);
        new_memory(&p, "another memory entirely\n");
        link_old_to_new(&p);
        pass(&h);
        assert!(new_copy(&h).unwrap().contains("another memory entirely"));
        let texts = vault_texts(&h);
        assert!(!texts.iter().any(|t| t.contains("one first")), "{texts:?}");
    }

    #[test]
    fn an_import_between_a_move_and_the_first_follow_copies_the_project_again() {
        let h = Home::new();
        h.init();
        let p = Projects::new();
        p.write("one", "one first");
        import(&h, p.path());
        fs::create_dir_all(p.path().join("-new")).unwrap();
        fs::rename(p.path().join("-proj/memory"), p.path().join("-new/memory")).unwrap();
        symlink(p.path().join("-new/memory"), p.path().join("-proj/memory")).unwrap();
        import(&h, p.path());
        assert!(copy(&h, "one").is_some() && new_copy(&h).is_some());
    }

    #[test]
    fn a_removed_copy_is_made_again_after_a_link_is_undone() {
        let (h, p) = following(&["one"]);
        let source = fs::read(p.file("one")).unwrap();
        new_memory(&p, "another memory entirely\n");
        pass(&h);
        fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
        link_old_to_new(&p);
        pass(&h);
        pass(&h);
        fs::remove_file(p.path().join("-proj/memory")).unwrap();
        fs::create_dir_all(p.path().join("-proj/memory")).unwrap();
        fs::write(p.file("one"), source).unwrap();
        let said = pass(&h);
        assert!(copy(&h, "one").is_some(), "{said}");
    }

    /// `-proj` and `-zed` followed, each with its own `one`, then `-proj` linked to `-zed` with
    /// its copy removed by the person, or archived and its archived file deleted.
    fn linked_over_a_gone_duplicate(deleted_from_archive: bool) -> (Home, Projects) {
        let (h, p) = following(&["one"]);
        let zed = p.path().join("-zed/memory");
        fs::create_dir_all(&zed).unwrap();
        fs::write(zed.join("one.md"), "another memory entirely\n").unwrap();
        pass(&h);
        if !deleted_from_archive {
            fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
        }
        fs::remove_dir_all(p.path().join("-proj/memory")).unwrap();
        symlink(&zed, p.path().join("-proj/memory")).unwrap();
        pass(&h);
        if deleted_from_archive {
            fs::remove_file(archive_path(&h, "one")).unwrap();
        }
        pass(&h);
        pass(&h);
        (h, p)
    }

    /// The live copy is at `name` and takes its source's next words.
    fn follows(h: &Home, p: &Projects, name: &str) {
        let source = p.path().join(name).join("memory/one.md");
        pass(h);
        fs::write(&source, "changed words\n").unwrap();
        let said = pass(h);
        let copy = h
            .vault
            .path()
            .join("working-memory")
            .join(name)
            .join("one.md");
        let text = fs::read_to_string(&copy).unwrap_or_default();
        assert!(text.contains("changed words"), "{said}");
    }

    fn rename_zed_to(p: &Projects, name: &str) {
        let to = p.path().join(name);
        fs::create_dir_all(&to).unwrap();
        fs::rename(p.path().join("-zed/memory"), to.join("memory")).unwrap();
        symlink(to.join("memory"), p.path().join("-zed/memory")).unwrap();
        fs::remove_file(p.path().join("-proj/memory")).unwrap();
        symlink(to.join("memory"), p.path().join("-proj/memory")).unwrap();
    }

    #[test]
    fn a_second_rename_after_a_removed_duplicate_keeps_the_live_copy() {
        for deleted_from_archive in [false, true] {
            let (h, p) = linked_over_a_gone_duplicate(deleted_from_archive);
            rename_zed_to(&p, "-zz");
            follows(&h, &p, "-zz");
        }
    }

    #[test]
    fn a_rename_back_after_a_removed_duplicate_keeps_the_live_copy() {
        let (h, p) = linked_over_a_gone_duplicate(false);
        fs::remove_file(p.path().join("-proj/memory")).unwrap();
        fs::rename(p.path().join("-zed/memory"), p.path().join("-proj/memory")).unwrap();
        symlink(p.path().join("-proj/memory"), p.path().join("-zed/memory")).unwrap();
        follows(&h, &p, "-proj");
    }

    /// Every markdown file's text anywhere in the vault, the archive included.
    fn vault_texts(h: &Home) -> Vec<String> {
        let mut dirs = vec![h.vault.path().to_path_buf()];
        let mut texts = Vec::new();
        while let Some(dir) = dirs.pop() {
            for e in fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
                if e.is_dir() {
                    dirs.push(e);
                } else {
                    texts.push(fs::read_to_string(e).unwrap_or_default());
                }
            }
        }
        texts
    }
}
