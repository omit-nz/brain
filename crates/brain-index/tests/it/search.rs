use brain_index::{Found, GAP_FLOOR, Hit, Query, Scope, is_gap};
use brain_note::{Kind, Place};
use ll_core::embed::{EmbeddingProvider, ModelConfig};

use super::Fixture;

fn query(text: &str) -> Query<'_> {
    Query {
        text,
        limit: 10,
        kinds: None,
        scope: Scope::Mine,
        rerank: false,
    }
}

fn paths(hits: &[Hit]) -> Vec<&str> {
    hits.iter().map(|h| h.path.as_str()).collect()
}

/// Embeds by concept: words in one group share a vector, so "car" finds "automobile".
struct Concepts {
    config: ModelConfig,
}

impl Concepts {
    const GROUPS: &[&[&str]] = &[
        &["car", "automobile", "vehicle"],
        &["tea", "kettle"],
        &["ledger", "invoice"],
    ];

    fn new() -> Concepts {
        Concepts {
            config: ModelConfig::new("concepts".into(), Self::GROUPS.len() + 1, 512),
        }
    }
}

impl EmbeddingProvider for Concepts {
    fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; Self::GROUPS.len() + 1];
                for w in t.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
                    let g = Self::GROUPS.iter().position(|g| g.contains(&w));
                    v[g.unwrap_or(Self::GROUPS.len())] += 1.0;
                }
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
                v.iter().map(|x| x / n).collect()
            })
            .collect())
    }

    fn config(&self) -> &ModelConfig {
        &self.config
    }
}

#[test]
fn bm25_finds_a_title_word() {
    let f = Fixture::new();
    f.put(
        "a.md",
        "---\ntitle: Quarterly harvest\n---\nnotes on the season\n",
    );
    f.put("b.md", "unrelated text about the office\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let hits = idx.search(&query("harvest"), None).unwrap().hits;
    assert_eq!(paths(&hits), vec!["a.md"]);
    assert_eq!(hits[0].title, "Quarterly harvest");
    assert!(hits[0].note_uuid.is_some());
}

/// A row the final lookup cannot read is an error, not a hit quietly left out.
#[test]
fn a_row_the_lookup_cannot_read_is_an_error() {
    let f = Fixture::new();
    f.put("a.md", "---\ntitle: Quarterly harvest\n---\nnotes\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    rusqlite::Connection::open(f.db())
        .unwrap()
        .execute("UPDATE notes SET title = X'FF' WHERE path = 'a.md'", [])
        .unwrap();
    assert!(idx.search(&query("harvest"), None).is_err());
}

#[test]
fn a_vector_match_ranks_without_a_shared_word() {
    let f = Fixture::new();
    f.put("auto.md", "the automobile needs a service\n");
    f.put("tea.md", "put the kettle on\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let p = Concepts::new();
    idx.embed_pending(&p, 10).unwrap();
    assert_eq!(idx.search(&query("car"), None).unwrap().hits.len(), 0);
    let hits = idx.search(&query("car"), Some(&p)).unwrap().hits;
    assert_eq!(paths(&hits)[0], "auto.md");
}

#[test]
fn a_linked_note_rises_by_the_graph() {
    let f = Fixture::new();
    f.put("hub.md", "zephyr zephyr zephyr, see [[spoke]]\n");
    f.put("spoke.md", "a note with none of the words\n");
    f.put("other.md", "another note entirely\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let hits = idx.search(&query("zephyr"), None).unwrap().hits;
    assert_eq!(paths(&hits), vec!["hub.md", "spoke.md"]);
}

#[test]
fn rerank_reorders_when_asked() {
    assert!(
        std::env::var_os("ORT_DYLIB_PATH").is_some(),
        "set ORT_DYLIB_PATH (scripts/fetch-ort.sh); CI does"
    );
    let f = Fixture::new();
    f.put(
        "stuffed.md",
        "---\ntitle: France capital capital France\n---\ncapital France capital France, a list of words with no answer\n",
    );
    f.put(
        "answer.md",
        "Paris is the capital and largest city of France.\n",
    );
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let mut q = query("what is the capital of France");
    let plain = idx.search(&q, None).unwrap().hits;
    assert_eq!(paths(&plain)[0], "stuffed.md");
    q.rerank = true;
    let reranked = idx.search(&q, None).unwrap().hits;
    assert_eq!(paths(&reranked)[0], "answer.md");
}

#[test]
fn rerank_without_a_runtime_is_refused() {
    let f = Fixture::new();
    f.put("a.md", "words\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    // SAFETY: nextest runs each test in its own process, so no other thread reads the env.
    unsafe { std::env::remove_var("ORT_DYLIB_PATH") };
    let mut q = query("words");
    q.rerank = true;
    let err = format!("{:#}", idx.search(&q, None).err().unwrap());
    assert!(err.contains("ORT_DYLIB_PATH"), "{err}");
}

#[test]
fn shared_only_never_shows_private_or_inbox_notes() {
    let f = Fixture::new();
    for i in 0..5 {
        f.put(
            &format!("p{i}.md"),
            "---\nvisibility: private\n---\nmeridian meridian meridian meridian\n",
        );
    }
    for i in 0..2 {
        f.put(
            &format!("0-inbox/i{i}.md"),
            "meridian meridian meridian meridian\n",
        );
    }
    f.put("private/x.md", "meridian meridian meridian\n");
    f.put(
        "listed.md",
        "---\nvisibility: listed\n---\nmeridian meridian\n",
    );
    for i in 0..4 {
        f.put(
            &format!("s{i}.md"),
            &format!("meridian once, note {i}, see [[p0]]\n"),
        );
    }
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let p = Concepts::new();
    idx.embed_pending(&p, 50).unwrap();
    let mut q = query("meridian");
    q.limit = 3;
    q.scope = Scope::SharedOnly;
    let hits = idx.search(&q, Some(&p)).unwrap().hits;
    assert_eq!(hits.len(), 3);
    for h in &hits {
        assert!(
            h.path.starts_with('s'),
            "{} leaked into a shared-only search",
            h.path
        );
        assert_eq!(h.disclosure, brain_note::Disclosure::Shared);
    }
    q.scope = Scope::Mine;
    q.limit = 20;
    assert_eq!(idx.search(&q, Some(&p)).unwrap().hits.len(), 13);
}

#[test]
fn kinds_filter_works() {
    let f = Fixture::new();
    f.put("r.md", "---\nkind: rule\n---\nlantern rule\n");
    f.put("h.md", "---\nkind: history\n---\nlantern history\n");
    f.put("k.md", "lantern knowledge\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let mut q = query("lantern");
    q.kinds = Some(&[Kind::Rule, Kind::History]);
    let mut got = paths(&idx.search(&q, None).unwrap().hits)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    got.sort();
    assert_eq!(got, vec!["h.md", "r.md"]);
}

#[test]
fn fts_syntax_in_a_query_is_searched_as_words() {
    let f = Fixture::new();
    f.put("a.md", "a b x foo near title don't\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    for text in [
        "\"a b",
        "-x",
        "foo*",
        "NEAR(a b)",
        "title:x",
        "a AND",
        "don't",
        "",
        "   ",
    ] {
        assert!(idx.search(&query(text), None).is_ok(), "{text:?}");
    }
}

#[test]
fn search_without_a_model_still_answers() {
    let f = Fixture::new();
    f.put("a.md", "orchard pruning schedule\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let hits = idx.search(&query("pruning"), None).unwrap().hits;
    assert_eq!(paths(&hits), vec!["a.md"]);
    assert!(hits[0].snippet.contains("orchard pruning"));
}

#[test]
fn found_reports_whether_words_matched() {
    let f = Fixture::new();
    f.put("a.md", "orchard pruning schedule\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    assert!(idx.search(&query("orchard"), None).unwrap().words);
    let none = idx.search(&query("zeppelin"), None).unwrap();
    assert!(!none.words);
    assert_eq!(none.best_similarity, None);
}

#[test]
fn found_reports_the_best_similarity() {
    let f = Fixture::new();
    // Concept words only, so car sits near automobile and far from kettle.
    f.put(
        "auto.md",
        "---\ntitle: automobile\nvisibility: private\n---\nautomobile vehicle\n",
    );
    f.put(
        "tea.md",
        "---\ntitle: kettle\nvisibility: shared\n---\nkettle tea\n",
    );
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let p = Concepts::new();
    idx.embed_pending(&p, 10).unwrap();
    let mine = idx.search(&query("car"), Some(&p)).unwrap();
    assert!(
        mine.best_similarity.unwrap() > GAP_FLOOR,
        "{:?}",
        mine.best_similarity
    );
    let mut q = query("car");
    q.scope = Scope::SharedOnly;
    let shared = idx.search(&q, Some(&p)).unwrap();
    assert!(
        shared.best_similarity.unwrap() < GAP_FLOOR,
        "the private automobile note is out of scope: {:?}",
        shared.best_similarity
    );
}

#[test]
fn a_hit_carries_its_body_and_place() {
    let f = Fixture::new();
    f.put("0-inbox/x.md", "harvest notes from the north block\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    let hits = idx.search(&query("harvest"), None).unwrap().hits;
    assert_eq!(hits[0].place, Place::Inbox);
    assert!(hits[0].body.contains("north block"));
}

#[test]
fn is_gap_reads_the_floor_when_vectors_exist() {
    let found = |words, best_similarity| Found {
        hits: Vec::new(),
        words,
        best_similarity,
    };
    assert!(
        is_gap(&found(true, Some(0.2))),
        "words do not rescue a far vector"
    );
    assert!(!is_gap(&found(false, Some(0.9))));
    assert!(is_gap(&found(false, None)));
    assert!(!is_gap(&found(true, None)));
    assert!(is_gap(&found(true, Some(GAP_FLOOR - 0.01))));
    assert!(!is_gap(&found(false, Some(GAP_FLOOR))));
}
