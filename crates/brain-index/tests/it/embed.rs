use std::fs;
use std::sync::Mutex;

use brain_index::{HashEmbedder, Index, embed};
use brain_note::Vault;
use ll_core::embed::{EmbeddingProvider, ModelConfig};
use rusqlite::Connection;

struct Fixture {
    vault: tempfile::TempDir,
    data: tempfile::TempDir,
}

impl Fixture {
    fn with_notes(n: usize) -> Fixture {
        let f = Fixture {
            vault: tempfile::tempdir().unwrap(),
            data: tempfile::tempdir().unwrap(),
        };
        for i in 0..n {
            f.put(&format!("n{i:03}.md"), &format!("note number {i}\n"));
        }
        f
    }

    fn put(&self, rel: &str, text: &str) {
        fs::write(self.vault.path().join(rel), text).unwrap();
    }

    fn indexed(&self) -> Index {
        let mut idx = Index::open(&self.data.path().join("index.db")).unwrap();
        idx.reindex(&Vault::open(self.vault.path()).unwrap())
            .unwrap();
        idx
    }

    fn reindex(&self, idx: &mut Index) {
        idx.reindex(&Vault::open(self.vault.path()).unwrap())
            .unwrap();
    }

    fn sql(&self) -> Connection {
        Connection::open(self.data.path().join("index.db")).unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.sql().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

/// Fails every batch that holds a text containing `poison`, and records every batch it is given.
struct Picky {
    poison: &'static str,
    always: bool,
    batches: Mutex<Vec<Vec<String>>>,
    inner: HashEmbedder,
}

impl Picky {
    fn new(poison: &'static str, always: bool) -> Picky {
        Picky {
            poison,
            always,
            batches: Mutex::new(Vec::new()),
            inner: HashEmbedder::new(8),
        }
    }
}

impl EmbeddingProvider for Picky {
    fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.batches.lock().unwrap().push(texts.to_vec());
        if self.always {
            anyhow::bail!("the runtime is gone");
        }
        if texts.iter().any(|t| t.contains(self.poison)) {
            anyhow::bail!("cannot embed {}", self.poison);
        }
        self.inner.embed_batch(texts)
    }

    fn config(&self) -> &ModelConfig {
        self.inner.config()
    }
}

#[test]
fn pending_notes_are_embedded_in_batches() {
    let f = Fixture::with_notes(25);
    let mut idx = f.indexed();
    let p = Picky::new("never", false);
    let e = idx.embed_pending(&p, 10).unwrap();
    assert_eq!((e.done, e.failed, e.pending, e.stopped), (25, 0, 0, None));
    let sizes: Vec<usize> = p.batches.lock().unwrap().iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![10, 10, 5]);
    assert_eq!(f.count("SELECT count(*) FROM embeddings"), 25);
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE embedded = 1"), 25);
    let e = idx.embed_pending(&p, 10).unwrap();
    assert_eq!((e.done, e.pending), (0, 0));
}

#[test]
fn embed_batch_embeds_one_batch_and_counts_the_rest() {
    let f = Fixture::with_notes(25);
    let mut idx = f.indexed();
    let p = Picky::new("never", false);
    let e = idx.embed_batch(&p, 10).unwrap();
    assert_eq!((e.done, e.failed, e.pending), (10, 0, 15));
    let sizes: Vec<usize> = p.batches.lock().unwrap().iter().map(Vec::len).collect();
    assert_eq!(sizes, vec![10]);
}

#[test]
fn a_changed_note_is_embedded_again() {
    let f = Fixture::with_notes(3);
    let mut idx = f.indexed();
    idx.embed_pending(&HashEmbedder::new(8), 10).unwrap();
    let before: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT data FROM embeddings JOIN notes USING (id) WHERE path = 'n001.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let raw = fs::read_to_string(f.vault.path().join("n001.md")).unwrap();
    f.put(
        "n001.md",
        &raw.replace("note number 1", "an entirely new subject"),
    );
    f.reindex(&mut idx);
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE embedded = 0"), 1);
    let e = idx.embed_pending(&HashEmbedder::new(8), 10).unwrap();
    assert_eq!(e.done, 1);
    let after: Vec<u8> = f
        .sql()
        .query_row(
            "SELECT data FROM embeddings JOIN notes USING (id) WHERE path = 'n001.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(before, after);
}

#[test]
fn a_failed_embedder_marks_no_note() {
    let f = Fixture::with_notes(5);
    let mut idx = f.indexed();
    let e = idx.embed_pending(&Picky::new("", true), 2).unwrap();
    assert_eq!((e.done, e.failed, e.pending), (0, 0, 5));
    assert!(e.stopped.unwrap().contains("the runtime is gone"));
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE embedded != 0"), 0);
    assert_eq!(f.count("SELECT count(*) FROM skipped"), 0);
    let e = idx.embed_pending(&HashEmbedder::new(8), 2).unwrap();
    assert_eq!((e.done, e.pending, e.stopped), (5, 0, None));
}

#[test]
fn a_note_that_fails_alone_is_marked_and_the_rest_embed() {
    let f = Fixture::with_notes(300);
    let mut idx = f.indexed();
    let e = idx
        .embed_pending(&Picky::new("note number 137\n", false), 64)
        .unwrap();
    assert_eq!((e.done, e.failed, e.pending, e.stopped), (299, 1, 0, None));
    assert_eq!(
        f.count("SELECT count(*) FROM notes WHERE embedded = 1"),
        299
    );
    let (path, why): (String, String) = f
        .sql()
        .query_row(
            "SELECT path, why FROM skipped JOIN notes USING (path) WHERE embedded = -1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(path, "n137.md");
    assert!(why.contains("cannot embed"), "{why}");
}

/// A bad note alone in the last batch is the note's failure when the embedder still takes a
/// known-good text, so it is marked once instead of stopping every later run.
#[test]
fn a_bad_note_alone_in_the_last_batch_is_marked_not_blamed_on_the_embedder() {
    let f = Fixture::with_notes(33);
    let mut idx = f.indexed();
    let picky = Picky::new("note number 32\n", false);
    let e = idx.embed_pending(&picky, 32).unwrap();
    assert_eq!((e.done, e.failed, e.pending, e.stopped), (32, 1, 0, None));
    let again = idx.embed_pending(&picky, 32).unwrap();
    assert_eq!(
        (again.done, again.failed, again.pending, again.stopped),
        (0, 0, 0, None)
    );
}

#[test]
fn the_index_never_byte_cuts_embed_input() {
    let f = Fixture::with_notes(0);
    let body = "ü€😀 ".repeat(5000);
    f.put(
        "wide.md",
        &format!("---\ntitle: Wïde\ntags: [a, b]\n---\n{body}"),
    );
    let mut idx = f.indexed();
    let p = Picky::new("never", false);
    idx.embed_pending(&p, 10).unwrap();
    let batches = p.batches.lock().unwrap();
    assert_eq!(batches[0], vec![format!("Wïde\na b\n{body}")]);
}

#[test]
fn load_without_a_runtime_names_the_missing_file() {
    let missing = tempfile::tempdir()
        .unwrap()
        .path()
        .join("libonnxruntime.dylib");
    // SAFETY: nextest runs each test in its own process, so no other thread reads the env.
    unsafe { std::env::set_var("ORT_DYLIB_PATH", &missing) };
    let err = format!("{:#}", embed::load().err().unwrap());
    assert!(err.contains(&missing.display().to_string()), "{err}");
    unsafe { std::env::remove_var("ORT_DYLIB_PATH") };
    let err = format!("{:#}", embed::load().err().unwrap());
    assert!(err.contains("ORT_DYLIB_PATH"), "{err}");
}

/// A runtime file that cannot be loaded (truncated, the wrong platform, not a library) is named
/// at once; handed to the ort crate it would park the process for good.
#[test]
fn a_runtime_that_cannot_be_loaded_is_named_not_waited_on() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("libonnxruntime.dylib");
    fs::write(&bogus, "not a library").unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread reads the env.
    unsafe { std::env::set_var("ORT_DYLIB_PATH", &bogus) };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(embed::load().err().map(|e| format!("{e:#}")));
    });
    let err = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("embed::load waited on a runtime it cannot load")
        .expect("embed::load accepted a file that is not a library");
    assert!(err.contains("cannot be loaded"), "{err}");
    assert!(err.contains(&bogus.display().to_string()), "{err}");
}

#[test]
fn the_hash_embedder_is_deterministic_and_sized() {
    let h = HashEmbedder::new(16);
    let texts = vec![
        "alpha beta".to_string(),
        "alpha beta".to_string(),
        "gamma".to_string(),
    ];
    let v = h.embed_batch(&texts).unwrap();
    assert_eq!(v[0], v[1]);
    assert_ne!(v[0], v[2]);
    assert!(v.iter().all(|x| x.len() == 16));
}

fn embedded(f: &Fixture) -> Vec<String> {
    let c = f.sql();
    let mut st = c
        .prepare("SELECT path FROM notes JOIN embeddings USING (id) ORDER BY path")
        .unwrap();
    st.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// A release that changes the reading rules rereads every note. A vector is made from the title,
/// tags and body, so a note whose three read the same keeps it.
#[test]
fn a_reread_under_new_rules_keeps_the_vectors_of_unchanged_text() {
    let f = Fixture::with_notes(3);
    let mut idx = f.indexed();
    idx.embed_pending(&HashEmbedder::new(8), 10).unwrap();
    f.sql()
        .execute("UPDATE meta SET value = '0' WHERE key = 'rules'", [])
        .unwrap();
    // n001 read differently under the old rules, so its vector is of other text.
    f.sql()
        .execute(
            "UPDATE notes_content SET title = 'as the old rules read it'
             WHERE id = (SELECT id FROM notes WHERE path = 'n001.md')",
            [],
        )
        .unwrap();
    f.reindex(&mut idx);
    assert_eq!(embedded(&f), ["n000.md", "n002.md"]);
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE embedded = 1"), 2);
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE embedded = 0"), 1);
}

#[test]
fn an_edit_outside_the_embedded_text_keeps_the_vector() {
    let f = Fixture::with_notes(1);
    let mut idx = f.indexed();
    idx.embed_pending(&HashEmbedder::new(8), 10).unwrap();
    let raw = fs::read_to_string(f.vault.path().join("n000.md")).unwrap();
    f.put("n000.md", &raw.replacen("---\n", "---\nowner: sam\n", 1));
    f.reindex(&mut idx);
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE owner = 'sam'"), 1);
    assert_eq!(embedded(&f), ["n000.md"]);
    assert_eq!(f.count("SELECT count(*) FROM notes WHERE embedded = 1"), 1);
}

/// A note saved while thousands are pending is embedded in the next batch, and so is a note still
/// being typed, which is stored with no mtime.
#[test]
fn the_newest_notes_are_embedded_first() {
    let f = Fixture::with_notes(4);
    let at = |name: &str, secs: u64| {
        fs::File::options()
            .write(true)
            .open(f.vault.path().join(name))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
            .unwrap();
    };
    at("n000.md", 1_000);
    at("n001.md", 3_000);
    at("n002.md", 2_000);
    at("n003.md", 500);
    let mut idx = f.indexed();
    f.sql()
        .execute("UPDATE notes SET mtime_ns = -1 WHERE path = 'n003.md'", [])
        .unwrap();
    // Writing the ids saved each note again, so the mtimes are set back before ordering.
    for (name, secs) in [
        ("n000.md", 1_000_i64),
        ("n001.md", 3_000),
        ("n002.md", 2_000),
    ] {
        f.sql()
            .execute(
                "UPDATE notes SET mtime_ns = ?1 WHERE path = ?2",
                rusqlite::params![secs * 1_000_000_000, name],
            )
            .unwrap();
    }
    idx.embed_batch(&HashEmbedder::new(8), 2).unwrap();
    assert_eq!(embedded(&f), ["n001.md", "n003.md"]);
}
