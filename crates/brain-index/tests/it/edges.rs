use std::fs;

use brain_index::Index;

use super::{Fixture, ID_A};

const ID_B: &str = "0195f2e0-0000-7000-8000-00000000000b";

fn skipped_rows(f: &Fixture) -> Vec<(String, String)> {
    let c = f.sql();
    let mut st = c
        .prepare("SELECT path, why FROM skipped ORDER BY path")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[cfg(unix)]
#[test]
fn an_unwritable_note_is_indexed_without_an_id() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.put("ro/a.md", "read only\n");
    f.put("b.md", "writable\n");
    let ro = f.vault.path().join("ro");
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    let mut idx = f.index();
    let r = idx.reindex(&f.vault());
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    let r = r.unwrap();
    assert_eq!(r.unidentified, vec!["ro/a.md"]);
    assert_eq!(idx.note_count().unwrap(), 2);
    assert!(f.uuid_of("b.md").is_some());
}

#[test]
fn a_fixed_file_leaves_the_skipped_list() {
    let f = Fixture::new();
    fs::write(f.vault.path().join("bin.md"), [0xff, 0xfe]).unwrap();
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    assert_eq!(skipped_rows(&f).len(), 1);
    f.put("bin.md", "now text\n");
    idx.reindex(&f.vault()).unwrap();
    assert_eq!(skipped_rows(&f), vec![]);
}

#[test]
fn an_embed_failure_row_survives_an_unrelated_reindex() {
    let f = Fixture::new();
    f.put("a.md", "alpha\n");
    f.put("b.md", "beta\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let c = f.sql();
    c.execute("UPDATE notes SET embedded = -1 WHERE path = 'a.md'", [])
        .unwrap();
    c.execute(
        "INSERT INTO skipped(path, why) VALUES ('a.md', 'embedding failed: x')",
        [],
    )
    .unwrap();
    f.put("b.md", "beta changed\n");
    idx.reindex(&f.vault()).unwrap();
    assert_eq!(
        skipped_rows(&f),
        vec![("a.md".into(), "embedding failed: x".into())]
    );
    let raw = f.read("a.md");
    f.put("a.md", &raw.replace("alpha", "alpha changed"));
    idx.reindex(&f.vault()).unwrap();
    assert_eq!(skipped_rows(&f), vec![]);
}

#[test]
fn racing_openers_share_one_migrated_index() {
    let f = Fixture::new();
    let db = f.db();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..6).map(|_| s.spawn(|| Index::open(&db))).collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }
    });
    let v: String = f
        .sql()
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, "1");
}

#[test]
fn a_changed_note_loses_its_embedding() {
    let f = Fixture::new();
    f.put("a.md", "alpha\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let c = f.sql();
    c.execute(
        "INSERT INTO embeddings(id, model, data) SELECT id, 'm', x'00' FROM notes",
        [],
    )
    .unwrap();
    c.execute("UPDATE notes SET embedded = 1", []).unwrap();
    let raw = f.read("a.md");
    f.put("a.md", &raw.replace("alpha", "omega"));
    idx.reindex(&f.vault()).unwrap();
    assert_eq!(f.count("SELECT count(*) FROM embeddings"), 0);
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE embedded = 0"), 1);
}

#[test]
fn a_changed_note_replaces_its_links() {
    let f = Fixture::new();
    f.put("a.md", "see [[b]]\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let raw = f.read("a.md");
    f.put("a.md", &raw.replace("[[b]]", "[[c]]"));
    idx.reindex(&f.vault()).unwrap();
    let c = f.sql();
    let mut st = c.prepare("SELECT target FROM links").unwrap();
    let links: Vec<String> = st
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(links, vec!["c"]);
}

#[test]
fn two_notes_can_swap_ids() {
    let f = Fixture::new();
    f.put("a.md", &format!("---\nid: {ID_A}\n---\nfirst\n"));
    f.put("b.md", &format!("---\nid: {ID_B}\n---\nsecond\n"));
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    f.put("a.md", &format!("---\nid: {ID_B}\n---\nfirst, edited\n"));
    f.put("b.md", &format!("---\nid: {ID_A}\n---\nsecond, edited\n"));
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.changed, r.ids_reassigned), (2, 0));
    assert_eq!(f.uuid_of("a.md").unwrap(), ID_B);
    assert_eq!(f.uuid_of("b.md").unwrap(), ID_A);
}

#[cfg(unix)]
#[test]
fn a_note_that_fails_to_read_keeps_its_row() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.put("a.md", "alpha\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let path = f.vault.path().join("a.md");
    let raw = f.read("a.md");
    f.put("a.md", &raw.replace("alpha", "alpha, edited"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let r = idx.reindex(&f.vault());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let r = r.unwrap();
    assert_eq!(r.removed, 0);
    assert_eq!(r.skipped.len(), 1, "{:?}", r.skipped);
    assert_eq!(idx.note_count().unwrap(), 1);
    assert_eq!(f.fts("alpha"), vec!["a.md"]);
}

#[cfg(unix)]
#[test]
fn an_existing_index_file_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    drop(f.index());
    fs::set_permissions(f.db(), fs::Permissions::from_mode(0o644)).unwrap();
    drop(f.index());
    let mode = fs::metadata(f.db()).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "{mode:o}");
}

#[test]
fn stats_count_every_state() {
    let f = Fixture::new();
    for i in 0..4 {
        f.put(&format!("n{i}.md"), &format!("note {i}\n"));
    }
    f.put("broken.md", "---\nnever closed\n");
    fs::write(f.vault.path().join("bin.md"), [0xff]).unwrap();
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let c = f.sql();
    c.execute(
        "UPDATE notes SET embedded = 1 WHERE path IN ('n0.md', 'n1.md')",
        [],
    )
    .unwrap();
    c.execute("UPDATE notes SET embedded = -1 WHERE path = 'n2.md'", [])
        .unwrap();
    c.execute(
        "INSERT INTO skipped(path, why) VALUES ('n2.md', 'embedding failed: x')",
        [],
    )
    .unwrap();
    let s = idx.stats().unwrap();
    assert_eq!(
        (s.notes, s.embedded, s.failed, s.pending, s.unidentified),
        (5, 2, 1, 2, 1)
    );
    let skipped: Vec<(&str, usize)> = s.skipped.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    assert_eq!(skipped, vec![("embedding failed", 1), ("not utf-8", 1)]);
}

#[test]
fn path_of_finds_a_note_by_id() {
    let f = Fixture::new();
    f.put("a.md", &format!("---\nid: {ID_A}\n---\nx\n"));
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let id: uuid::Uuid = ID_A.parse().unwrap();
    assert_eq!(idx.path_of(&id).unwrap().as_deref(), Some("a.md"));
    assert_eq!(idx.path_of(&uuid::Uuid::nil()).unwrap(), None);
}
