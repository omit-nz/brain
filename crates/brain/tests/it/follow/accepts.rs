//! A record accepts the hash brain is about to write and the one before it, so a rewrite cut
//! short between the record and the file leaves nothing that reads as the person's.

use std::time::SystemTime;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;

const LEDGER: &str = "data/import-ledger.json";
const ARCHIVE: &str = "data/follow-archive.json";
const KEY: &str = "memory/-proj/one.md";
/// The hash of bytes brain meant to write and never did.
const UNWRITTEN: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn read(h: &Home, rel: &str) -> Value {
    serde_json::from_str(&fs::read_to_string(h.home.path().join(rel)).unwrap()).unwrap()
}

fn write(h: &Home, rel: &str, value: &Value) {
    fs::write(h.home.path().join(rel), value.to_string()).unwrap();
}

/// Moves a record on to bytes never written, still accepting the file as it is.
fn move_on(record: &mut Value, file: &Path) {
    record["was"] = json!(sha256(&fs::read(file).unwrap()));
    record["dst_hash"] = json!(UNWRITTEN);
}

fn move_ledger_on(h: &Home, file: &Path) {
    let mut ledger = read(h, LEDGER);
    move_on(&mut ledger[KEY], file);
    write(h, LEDGER, &ledger);
}

fn move_archive_on(h: &Home, file: &Path) {
    let mut records = read(h, ARCHIVE);
    move_on(&mut records[0], file);
    write(h, ARCHIVE, &records);
}

fn archive_path(h: &Home) -> PathBuf {
    h.vault.path().join("_archive/working-memory/-proj/one.md")
}

/// `one` followed beside `stays`, so removing `one` never leaves the folder empty.
fn two() -> (Home, Projects) {
    following(&["one", "stays"])
}

/// Removes `one`'s source and looks twice, which archives its copy.
fn archive_one(h: &Home, p: &Projects) -> String {
    p.remove("one");
    pass(h);
    pass(h)
}

#[test]
fn an_unwritten_copy_takes_its_sources_change() {
    let (h, p) = two();
    move_ledger_on(&h, &copy_path(h.vault.path(), "one"));
    p.write("one", "one second");
    let said = pass(&h);
    assert!(said.contains("1 updated"), "{said}");
    assert!(copy(&h, "one").unwrap().contains("one second"));
}

#[test]
fn a_copy_the_person_edited_is_still_theirs() {
    let (h, p) = two();
    let path = copy_path(h.vault.path(), "one");
    move_ledger_on(&h, &path);
    fs::write(&path, "mine\n").unwrap();
    p.write("one", "one second");
    let said = pass(&h);
    assert!(said.contains("1 kept as they are"), "{said}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "mine\n");
}

#[test]
fn an_unwritten_copy_is_given_its_sources_time() {
    let (h, p) = two();
    let path = copy_path(h.vault.path(), "one");
    let modified = |at: &Path| -> SystemTime { fs::metadata(at).unwrap().modified().unwrap() };
    crate::set_changed(&path, "2027-01-01T00:00:00Z");
    move_ledger_on(&h, &path);
    pass(&h);
    assert_eq!(modified(&path), modified(&p.file("one")));
}

#[test]
fn an_unwritten_copy_of_a_gone_source_is_archived_with_both_hashes() {
    let (h, p) = two();
    let path = copy_path(h.vault.path(), "one");
    let was = sha256(&fs::read(&path).unwrap());
    move_ledger_on(&h, &path);
    let said = archive_one(&h, &p);
    assert!(said.contains("1 archived"), "{said}");
    let record = &read(&h, ARCHIVE)[0];
    assert_eq!(record["dst_hash"], UNWRITTEN);
    assert_eq!(record["was"], was.as_str());
}

#[test]
fn an_unwritten_file_put_back_by_hand_is_the_archived_one() {
    let (h, p) = two();
    archive_one(&h, &p);
    let back = copy_path(h.vault.path(), "one");
    fs::copy(archive_path(&h), &back).unwrap();
    move_archive_on(&h, &back);
    pass(&h);
    assert_eq!(read(&h, ARCHIVE).as_array().unwrap().len(), 0);
    let entry = &read(&h, LEDGER)[KEY];
    assert_eq!(entry["was"], sha256(&fs::read(&back).unwrap()).as_str());
}

#[test]
fn a_restored_unwritten_copy_is_settled_as_it_reads() {
    let (h, p) = two();
    let source = fs::read(p.file("one")).unwrap();
    archive_one(&h, &p);
    move_archive_on(&h, &archive_path(&h));
    fs::write(p.file("one"), source).unwrap();
    let said = pass(&h);
    assert!(said.contains("1 restored"), "{said}");
    let bytes = fs::read(copy_path(h.vault.path(), "one")).unwrap();
    assert_eq!(read(&h, LEDGER)[KEY]["dst_hash"], sha256(&bytes).as_str());
}

/// The move was made and the ledger not yet saved, then a rewrite stopped between the two files.
#[test]
fn a_record_survives_a_ledger_that_moved_on_without_it() {
    let (h, p) = two();
    let mut entry = read(&h, LEDGER)[KEY].clone();
    archive_one(&h, &p);
    move_on(&mut entry, &archive_path(&h));
    let mut ledger = read(&h, LEDGER);
    ledger[KEY] = entry;
    write(&h, LEDGER, &ledger);
    pass(&h);
    assert_eq!(read(&h, ARCHIVE).as_array().unwrap().len(), 1);
    assert_eq!(read(&h, LEDGER)[KEY], Value::Null);
}
