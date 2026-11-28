//! What a note's own dates and declarations do to a search: asking for the latest, and notes
//! another one corrects or supersedes. Ranking needs the cross-encoder, so ONNX Runtime.

use brain_index::{Found, Hit, Query, Scope, is_gap};
use jiff::civil::{Date, date};

use super::Fixture;

const TODAY: Date = date(2026, 11, 28);

/// The older note answers the question in its own words; the newer one only in substance.
fn pricing(f: &Fixture) {
    f.put(
        "pricing.md",
        "---\ntitle: Omit pricing decision\ndate: 2026-10-27\n---\nThe decision on omit pricing: omit pricing is a share of sales for store migrations.\n",
    );
    f.put(
        "working-memory/p/direction.md",
        "2026-11-24 omit pricing decision: setup fee plus flat monthly care, on every plan.\n",
    );
}

fn query(text: &str) -> Query<'_> {
    Query {
        text,
        limit: 10,
        kinds: None,
        scope: Scope::Mine,
        rerank: true,
        today: TODAY,
    }
}

fn search(f: &Fixture, q: &Query<'_>) -> Found {
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    idx.search(q, None).unwrap()
}

fn ask(f: &Fixture, text: &str) -> Found {
    search(f, &query(text))
}

/// A search by words alone, where every note holding them is listed.
fn words(f: &Fixture, text: &str) -> Found {
    search(
        f,
        &Query {
            rerank: false,
            ..query(text)
        },
    )
}

fn paths(found: &Found) -> Vec<&str> {
    found.hits.iter().map(|h| h.path.as_str()).collect()
}

fn hit<'a>(found: &'a Found, path: &str) -> &'a Hit {
    found
        .hits
        .iter()
        .find(|h| h.path == path)
        .unwrap_or_else(|| panic!("no {path} in {:?}", paths(found)))
}

#[test]
fn a_query_for_the_latest_orders_by_date() {
    let f = Fixture::new();
    pricing(&f);
    assert_eq!(
        paths(&ask(&f, "decision on omit pricing")),
        ["pricing.md", "working-memory/p/direction.md"]
    );
    let latest = ask(&f, "latest decision on omit pricing");
    assert_eq!(
        paths(&latest),
        ["working-memory/p/direction.md", "pricing.md"]
    );
    assert_eq!(latest.hits[0].said, Some(date(2026, 11, 24)));
}

#[test]
fn undated_hits_keep_their_places() {
    let f = Fixture::new();
    pricing(&f);
    f.put(
        "undated.md",
        "---\ntitle: Omit pricing decision\n---\nThe decision on omit pricing: omit pricing is under review.\n",
    );
    let latest = ask(&f, "latest decision on omit pricing");
    // Its place is the one its score gives it.
    let at = paths(&latest).iter().position(|p| *p == "undated.md");
    let score = hit(&latest, "undated.md").score;
    let above = latest.hits.iter().filter(|h| h.score > score).count();
    assert_eq!(at, Some(above), "{:?}", paths(&latest));
    let dated: Vec<&str> = paths(&latest)
        .into_iter()
        .filter(|p| *p != "undated.md")
        .collect();
    assert_eq!(dated, ["working-memory/p/direction.md", "pricing.md"]);
}

#[test]
fn a_current_question_keeps_score_order() {
    let f = Fixture::new();
    f.put(
        "seats-old.md",
        "---\ntitle: Claude seat setup\ndate: 2026-10-01\n---\nThe current seat setup for Claude is four Max seats, one per account.\n",
    );
    f.put(
        "seats-new.md",
        "---\ntitle: Seat review\ndate: 2026-11-20\n---\nA review of how seats are used: the current setup works.\n",
    );
    let found = ask(&f, "how does the current Claude seat setup work");
    assert_eq!(paths(&found), ["seats-old.md", "seats-new.md"]);
}

#[test]
fn without_a_reranker_score_order_stands() {
    let f = Fixture::new();
    pricing(&f);
    assert_eq!(
        paths(&words(&f, "decision on omit pricing"))[0],
        "pricing.md"
    );
    // "latest" is in neither note, so the words fall back to any of them; the older still leads.
    assert_eq!(
        paths(&words(&f, "latest decision on omit pricing"))[0],
        "pricing.md"
    );
}

#[test]
fn a_future_date_counts_from_its_day() {
    let f = Fixture::new();
    f.put("call.md", "---\ndate: 2026-11-30\n---\nthe quince call\n");
    let on = |today| {
        let q = Query {
            rerank: false,
            today,
            ..query("quince")
        };
        search(&f, &q).hits[0].said
    };
    assert_eq!(on(date(2026, 11, 28)), None);
    assert_eq!(on(date(2026, 11, 30)), Some(date(2026, 11, 30)));
}

#[test]
fn a_file_time_never_dates_a_note() {
    let f = Fixture::new();
    f.put("a.md", "quince\n");
    let found = words(&f, "quince");
    assert_eq!(found.hits[0].said, None);
    assert!(found.hits[0].modified.is_some());
}

const OLD_ID: &str = "0195f2e0-0000-7000-8000-0000000000d1";

fn old_note(f: &Fixture) {
    f.put(
        "plan/old-plan.md",
        &format!("---\nid: {OLD_ID}\ndate: 2026-11-27\n---\npoint the quince at the copy\n"),
    );
}

#[test]
fn a_corrected_note_is_shown_overtaken_under_its_correction() {
    let f = Fixture::new();
    old_note(&f);
    // The same day, and sharing none of the words searched for.
    f.put(
        "0-inbox/correction.md",
        &format!("---\ndate: 2026-11-27\ncorrects: {OLD_ID}\n---\ndone already\n"),
    );
    let found = words(&f, "quince copy");
    assert_eq!(paths(&found), ["0-inbox/correction.md", "plan/old-plan.md"]);
    assert_eq!(
        hit(&found, "plan/old-plan.md").overtaken_by.as_deref(),
        Some("0-inbox/correction.md")
    );
    assert!(hit(&found, "0-inbox/correction.md").added);
    assert!(!hit(&found, "plan/old-plan.md").added);
}

#[test]
fn supersedes_takes_a_path_a_file_name_or_a_link() {
    for name in [
        "plan/old-plan.md",
        "old-plan.md",
        "old-plan",
        "\"[[old-plan]]\"",
        "[[old-plan]]",
        OLD_ID,
    ] {
        let f = Fixture::new();
        old_note(&f);
        f.put("new.md", &format!("---\nsupersedes: {name}\n---\nlater\n"));
        let found = words(&f, "quince");
        assert_eq!(paths(&found), ["new.md", "plan/old-plan.md"], "{name}");
    }
}

#[test]
fn supersedes_free_text_a_shared_stem_or_an_older_note_marks_nothing() {
    let marked = |f: &Fixture| {
        words(f, "quince")
            .hits
            .iter()
            .any(|h| h.overtaken_by.is_some())
    };
    for name in ["the old plan for quinces", "nothing; extends old-plan"] {
        let f = Fixture::new();
        old_note(&f);
        f.put("new.md", &format!("---\nsupersedes: {name}\n---\nlater\n"));
        assert!(!marked(&f), "{name}");
    }
    let f = Fixture::new();
    old_note(&f);
    f.put("other/old-plan.md", "another quince of that name\n");
    f.put("new.md", "---\nsupersedes: old-plan\n---\nlater\n");
    assert!(!marked(&f), "a stem two notes share");
    let f = Fixture::new();
    old_note(&f);
    f.put(
        "new.md",
        "---\ndate: 2026-01-01\nsupersedes: old-plan\n---\nearlier\n",
    );
    assert!(!marked(&f), "an older note");
}

#[test]
fn two_notes_naming_each_other_mark_neither() {
    let f = Fixture::new();
    f.put("a.md", "---\nsupersedes: b\n---\nquince\n");
    f.put("b.md", "---\nsupersedes: a\n---\nquince\n");
    let found = words(&f, "quince");
    assert!(found.hits.iter().all(|h| h.overtaken_by.is_none()));
}

#[test]
fn a_ring_of_three_is_listed_once_each() {
    let f = Fixture::new();
    f.put("a.md", "---\nsupersedes: b\n---\nquince\n");
    f.put("b.md", "---\nsupersedes: c\n---\npear\n");
    f.put("c.md", "---\nsupersedes: a\n---\nplum\n");
    let found = words(&f, "quince");
    let mut listed = paths(&found);
    listed.sort_unstable();
    assert_eq!(listed, ["a.md", "b.md", "c.md"]);
}

#[test]
fn an_overtaker_stays_within_the_limit_and_never_fills_a_gap() {
    let f = Fixture::new();
    old_note(&f);
    f.put("new.md", "---\nsupersedes: old-plan\n---\nlater\n");
    let one = search(
        &f,
        &Query {
            limit: 1,
            rerank: false,
            ..query("quince")
        },
    );
    assert_eq!(paths(&one), ["new.md"]);
    let none = ask(&f, "what is the capital of France");
    assert!(is_gap(&none), "{:?}", paths(&none));
    assert_eq!(paths(&none), Vec::<&str>::new());
}

#[test]
fn a_private_overtaker_is_not_shown_to_search_company() {
    let f = Fixture::new();
    f.put("old.md", "---\nvisibility: shared\n---\nthe quince rule\n");
    f.put(
        "new.md",
        "---\nvisibility: private\nsupersedes: old\n---\nsecret\n",
    );
    // A shared declaration, so the company view has some to read.
    f.put(
        "other.md",
        "---\nvisibility: shared\nsupersedes: nothing\n---\npear\n",
    );
    let q = Query {
        scope: Scope::SharedOnly,
        rerank: false,
        ..query("quince")
    };
    let found = search(&f, &q);
    assert_eq!(paths(&found), ["old.md"]);
    assert_eq!(found.hits[0].overtaken_by, None);
}

/// An index built before notes were dated rereads them, and keeps their vectors.
#[test]
fn an_existing_index_gets_its_dates_on_upgrade() {
    let f = Fixture::new();
    f.put("a.md", "2026-11-24 rulings: alpha\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    idx.embed_pending(&brain_index::HashEmbedder::new(8), 10)
        .unwrap();
    let c = f.sql();
    c.execute("UPDATE notes SET said = NULL", []).unwrap();
    // 3: the rules every index was built under before notes were dated.
    c.execute("UPDATE meta SET value = '3' WHERE key = 'rules'", [])
        .unwrap();
    idx.reindex(&f.vault()).unwrap();
    let said: String = c
        .query_row("SELECT said FROM notes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(said, "2026-11-24");
    assert_eq!(f.count("SELECT count(*) FROM embeddings"), 1);
}
