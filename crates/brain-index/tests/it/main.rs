use std::fs;
use std::path::{Path, PathBuf};

use brain_index::Index;
use brain_note::Vault;
use rusqlite::Connection;

struct Fixture {
    vault: tempfile::TempDir,
    data: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture {
            vault: tempfile::tempdir().unwrap(),
            data: tempfile::tempdir().unwrap(),
        }
    }

    fn put(&self, rel: &str, text: &str) {
        let p = self.vault.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.vault.path().join(rel)).unwrap()
    }

    fn db(&self) -> PathBuf {
        self.data.path().join("index.db")
    }

    fn index(&self) -> Index {
        Index::open(&self.db()).unwrap()
    }

    fn vault(&self) -> Vault {
        Vault::open(self.vault.path()).unwrap()
    }

    fn sql(&self) -> Connection {
        Connection::open(self.db()).unwrap()
    }

    fn uuid_of(&self, path: &str) -> Option<String> {
        self.sql()
            .query_row("SELECT note_uuid FROM notes WHERE path = ?1", [path], |r| {
                r.get(0)
            })
            .unwrap()
    }

    fn fts(&self, word: &str) -> Vec<String> {
        let c = self.sql();
        let mut st = c
            .prepare(
                "SELECT n.path FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid
                 WHERE notes_fts MATCH ?1 ORDER BY n.path",
            )
            .unwrap();
        st.query_map([word], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }
}

const ID_A: &str = "0195f2e0-0000-7000-8000-00000000000a";

#[test]
fn a_new_vault_indexes_every_note() {
    let f = Fixture::new();
    f.put(
        "a.md",
        "---\ntitle: Alpha\ntags: [x, y]\nkind: rule\n---\nalpha body\n",
    );
    f.put("0-inbox/b.md", "beta body\n");
    f.put("private/c.md", "gamma\n");
    f.put("listed.md", "---\nvisibility: listed\n---\ndelta\n");
    let mut idx = f.index();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.added, r.changed, r.removed), (4, 0, 0));
    assert_eq!(idx.note_count(), 4);
    let row: (String, String, String, String, String) = f
        .sql()
        .query_row(
            "SELECT title, tags, kind, place, disclosure FROM notes WHERE path = 'a.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "Alpha".into(),
            "[\"x\",\"y\"]".into(),
            "rule".into(),
            "kept".into(),
            "shared".into()
        )
    );
    let places: Vec<(String, String, String)> = {
        let c = f.sql();
        let mut st = c
            .prepare("SELECT path, place, disclosure FROM notes ORDER BY path")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(
        places,
        vec![
            ("0-inbox/b.md".into(), "inbox".into(), "private".into()),
            ("a.md".into(), "kept".into(), "shared".into()),
            ("listed.md".into(), "kept".into(), "listed".into()),
            ("private/c.md".into(), "private".into(), "private".into()),
        ]
    );
    assert_eq!(f.fts("alpha"), vec!["a.md"]);
}

#[test]
fn an_unchanged_note_is_not_rewritten() {
    let f = Fixture::new();
    f.put("a.md", "alpha\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let after_first = f.read("a.md");
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.added, r.changed, r.removed, r.ids_written), (0, 0, 0, 0));
    assert_eq!(f.read("a.md"), after_first);

    // Same bytes, new mtime: the hash says nothing changed.
    let path = f.vault.path().join("a.md");
    let later =
        fs::metadata(&path).unwrap().modified().unwrap() + std::time::Duration::from_secs(5);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.added, r.changed, r.removed), (0, 0, 0));
}

#[test]
fn a_changed_note_updates_its_fts_row() {
    let f = Fixture::new();
    f.put(
        "a.md",
        "---\nid: 0195f2e0-0000-7000-8000-000000000001\n---\nhorse\n",
    );
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    assert_eq!(f.fts("horse"), vec!["a.md"]);
    f.put(
        "a.md",
        "---\nid: 0195f2e0-0000-7000-8000-000000000001\n---\nzebra crossing\n",
    );
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.added, r.changed), (0, 1));
    assert_eq!(f.fts("horse"), Vec::<String>::new());
    assert_eq!(f.fts("zebra"), vec!["a.md"]);
}

#[test]
fn a_deleted_file_leaves_the_index() {
    let f = Fixture::new();
    f.put("a.md", "apple\n");
    f.put("b.md", "banana\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    fs::remove_file(f.vault.path().join("a.md")).unwrap();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!(r.removed, 1);
    assert_eq!(idx.note_count(), 1);
    assert_eq!(f.fts("apple"), Vec::<String>::new());
    let content: i64 = f
        .sql()
        .query_row("SELECT count(*) FROM notes_content", [], |r| r.get(0))
        .unwrap();
    assert_eq!(content, 1);
}

#[test]
fn a_missing_id_is_written_once() {
    let f = Fixture::new();
    f.put("a.md", "---\ntitle: t\n---\nbody\n");
    let mut idx = f.index();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!(r.ids_written, 1);
    let uuid = f.uuid_of("a.md").unwrap();
    assert_eq!(
        f.read("a.md"),
        format!("---\ntitle: t\nid: {uuid}\n---\nbody\n")
    );
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.ids_written, r.changed), (0, 0));
    assert_eq!(f.uuid_of("a.md").unwrap(), uuid);
}

#[test]
fn the_held_id_wins_a_duplicate() {
    let f = Fixture::new();
    let text = format!("---\nid: {ID_A}\n---\nheld\n");
    f.put("z.md", &text);
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    f.put("a.md", &text);
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!(r.ids_reassigned, 1);
    assert_eq!(f.uuid_of("z.md").unwrap(), ID_A);
    let copy = f.uuid_of("a.md").unwrap();
    assert_ne!(copy, ID_A);
    assert!(f.read("a.md").contains(&format!("id: {copy}")));
    assert_eq!(f.read("z.md"), text);
}

#[test]
fn two_new_notes_with_one_id_split_by_walk_order() {
    let f = Fixture::new();
    let text = format!("---\nid: {ID_A}\n---\nsame\n");
    f.put("b.md", &text);
    f.put("a.md", &text);
    let mut idx = f.index();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!(r.ids_reassigned, 1);
    assert_eq!(f.uuid_of("a.md").unwrap(), ID_A);
    assert_ne!(f.uuid_of("b.md").unwrap(), ID_A);
}

#[test]
fn a_moved_note_keeps_its_id() {
    let f = Fixture::new();
    let text = format!("---\nid: {ID_A}\n---\nmoving\n");
    f.put("z.md", &text);
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    fs::rename(f.vault.path().join("z.md"), f.vault.path().join("a.md")).unwrap();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.added, r.removed, r.ids_reassigned), (1, 1, 0));
    assert_eq!(f.uuid_of("a.md").unwrap(), ID_A);
}

#[test]
fn a_note_without_an_id_is_indexed_and_reported() {
    let f = Fixture::new();
    let raw = "---\ntitle: never closed\nwalrus\n";
    f.put("broken.md", raw);
    let mut idx = f.index();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!(r.unidentified, vec!["broken.md"]);
    assert_eq!(f.uuid_of("broken.md"), None);
    assert_eq!(f.fts("walrus"), vec!["broken.md"]);
    assert_eq!(f.read("broken.md"), raw);
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!(r.unidentified, vec!["broken.md"]);
}

#[test]
fn skipped_files_are_reported_and_recorded() {
    let f = Fixture::new();
    f.put("ok.md", "fine\n");
    fs::write(f.vault.path().join("bin.md"), [0xff, 0xfe, 0x00]).unwrap();
    let mut idx = f.index();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!(
        r.skipped,
        vec![("bin.md".to_string(), brain_note::Skip::NotUtf8)]
    );
    assert_eq!(idx.note_count(), 1);
    let why: String = f
        .sql()
        .query_row("SELECT why FROM skipped WHERE path = 'bin.md'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!why.is_empty());
}

fn rows(c: &Connection) -> Vec<Vec<String>> {
    let mut st = c
        .prepare(
            "SELECT note_uuid, path, hex(hash), size, mtime_ns, title, tags, kind, place,
                    disclosure, owner, review
             FROM notes ORDER BY path",
        )
        .unwrap();
    st.query_map([], |r| {
        (0..12)
            .map(|i| r.get_ref(i).map(|v| format!("{v:?}")))
            .collect()
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

#[test]
fn the_index_rebuilds_from_the_vault_alone() {
    let f = Fixture::new();
    f.put(
        "a.md",
        "---\ntitle: A\nowner: robin\nreview: 2027-02-22\n---\nalpha [[b]]\n",
    );
    f.put("b.md", "beta\n");
    f.put("0-inbox/c.md", "gamma\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let before = rows(&f.sql());
    drop(idx);
    for entry in fs::read_dir(f.data.path()).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let mut idx = f.index();
    let r = idx.reindex(&f.vault()).unwrap();
    assert_eq!((r.added, r.ids_written), (3, 0));
    assert_eq!(rows(&f.sql()), before);
    let links: Vec<String> = {
        let c = f.sql();
        let mut st = c.prepare("SELECT target FROM links").unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(links, vec!["b"]);
}

/// Run only as a child of `a_killed_reindex_resumes_clean`, which sets the paths and the abort.
#[test]
#[ignore]
fn reindex_child() {
    let vault = std::env::var_os("BRAIN_TEST_VAULT").unwrap();
    let db = std::env::var_os("BRAIN_TEST_DB").unwrap();
    let mut idx = Index::open(Path::new(&db)).unwrap();
    idx.reindex(&Vault::open(Path::new(&vault)).unwrap())
        .unwrap();
}

#[test]
fn a_killed_reindex_resumes_clean() {
    let f = Fixture::new();
    for i in 0..450 {
        f.put(&format!("n{i:03}.md"), &format!("note number {i}\n"));
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["reindex_child", "--exact", "--ignored", "--nocapture"])
        .env("BRAIN_TEST_VAULT", f.vault.path())
        .env("BRAIN_TEST_DB", f.db())
        .env("BRAIN_TEST_ABORT_AFTER_NOTES", "250")
        .status()
        .unwrap();
    assert!(!status.success(), "the child was meant to abort");
    let committed: i64 = f
        .sql()
        .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(committed, 200, "only whole batches survive the abort");

    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    assert_eq!(idx.note_count(), 450);
    let c = f.sql();
    let content: i64 = c
        .query_row("SELECT count(*) FROM notes_content", [], |r| r.get(0))
        .unwrap();
    assert_eq!(content, 450);
    c.execute(
        "INSERT INTO notes_fts(notes_fts) VALUES('integrity-check')",
        [],
    )
    .unwrap();
}

#[test]
fn a_newer_schema_is_refused() {
    let f = Fixture::new();
    drop(f.index());
    f.sql()
        .execute(
            "UPDATE meta SET value = '99' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
    let err = format!("{:#}", Index::open(&f.db()).err().unwrap());
    assert!(
        err.contains("schema version 99") && err.contains("brain's 1"),
        "{err}"
    );
}

#[test]
fn the_index_file_is_private() {
    let f = Fixture::new();
    f.put("a.md", "x\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    assert!(f.db().exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in fs::read_dir(f.data.path()).unwrap() {
            let p = entry.unwrap().path();
            let mode = fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{} is {mode:o}", p.display());
        }
    }
}
