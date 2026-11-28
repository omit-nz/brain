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
fn an_added_overtaker_raises_none_of_its_own() {
    let f = Fixture::new();
    f.put("a.md", "quince\n");
    f.put("b.md", "---\nsupersedes: a\n---\npear\n");
    f.put("c.md", "---\nsupersedes: b\n---\nplum\n");
    let found = words(&f, "quince");
    assert_eq!(paths(&found), ["b.md", "a.md"]);
    assert!(found.hits[0].added);
    assert_eq!(found.hits[0].overtaken_by.as_deref(), Some("c.md"));
}

#[test]
fn a_ring_of_judged_notes_ends() {
    let f = Fixture::new();
    f.put("a.md", "---\nsupersedes: b\n---\nquince\n");
    f.put("b.md", "---\nsupersedes: c\n---\nquince\n");
    f.put("c.md", "---\nsupersedes: a\n---\nquince\n");
    let found = words(&f, "quince");
    let mut listed = paths(&found);
    listed.sort_unstable();
    assert_eq!(listed, ["a.md", "b.md", "c.md"]);
}

#[test]
fn a_ring_of_three_is_listed_once_each() {
    let f = Fixture::new();
    f.put("a.md", "---\nsupersedes: b\n---\nquince\n");
    f.put("b.md", "---\nsupersedes: c\n---\npear\n");
    f.put("c.md", "---\nsupersedes: a\n---\nplum\n");
    // Only a.md is judged, so only the note over it is added.
    assert_eq!(paths(&words(&f, "quince")), ["c.md", "a.md"]);
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
    let question = "who won the football world cup in the year of the quince";
    assert_eq!(paths(&words(&f, question)), ["new.md", "plan/old-plan.md"]);
    let none = ask(&f, question);
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

/// An index built before notes were dated gains the columns, rereads and keeps its vectors.
#[test]
fn an_existing_index_gets_its_dates_on_upgrade() {
    let f = Fixture::new();
    f.put("a.md", "2026-11-24 rulings: alpha\n");
    let mut idx = f.index();
    idx.reindex(&f.vault()).unwrap();
    idx.embed_pending(&brain_index::HashEmbedder::new(8), 10)
        .unwrap();
    drop(idx);
    // As 0.2.5 left it: schema 1, and the rules every index was built under before dates.
    let c = f.sql();
    for column in ["said", "corrects", "supersedes"] {
        c.execute(&format!("ALTER TABLE notes DROP COLUMN {column}"), [])
            .unwrap();
    }
    c.execute_batch(
        "UPDATE meta SET value = '1' WHERE key = 'schema_version';
         UPDATE meta SET value = '3' WHERE key = 'rules';",
    )
    .unwrap();
    brain_index::db::open(&f.db(), &brain_index::db::INDEX_MIGRATIONS[..1]).unwrap();
    f.index().reindex(&f.vault()).unwrap();
    let said: String = c
        .query_row("SELECT said FROM notes_shared", [], |r| r.get(0))
        .unwrap();
    assert_eq!(said, "2026-11-24");
    assert_eq!(f.count("SELECT count(*) FROM embeddings"), 1);
    assert_eq!(f.count("SELECT embedded FROM notes"), 1);
}

/// Each one-name declaration in `new.md` against `old_note`'s neighbours, and whether any is marked.
fn marked(f: &Fixture) -> Vec<String> {
    let mut over: Vec<String> = words(f, "quince")
        .hits
        .iter()
        .filter(|h| h.overtaken_by.is_some())
        .map(|h| h.path.clone())
        .collect();
    over.sort();
    over
}

#[test]
fn an_exact_path_wins_over_a_shared_file_name() {
    let f = Fixture::new();
    f.put("plan.md", "the quince plan\n");
    f.put("x/plan.md", "another quince plan\n");
    f.put("fix.md", "---\ncorrects: plan.md\n---\nfixed\n");
    assert_eq!(marked(&f), ["plan.md"]);
}

#[test]
fn corrects_takes_an_id_or_a_path_only() {
    for name in [
        "old-plan",
        "old-plan.md",
        "[[old-plan]]",
        "\"[[old-plan]]\"",
    ] {
        let f = Fixture::new();
        old_note(&f);
        f.put("new.md", &format!("---\ncorrects: {name}\n---\nlater\n"));
        assert_eq!(marked(&f), Vec::<String>::new(), "{name}");
    }
    for name in [OLD_ID, "plan/old-plan.md"] {
        let f = Fixture::new();
        old_note(&f);
        f.put("new.md", &format!("---\ncorrects: {name}\n---\nlater\n"));
        assert_eq!(marked(&f), ["plan/old-plan.md"], "{name}");
    }
}

#[test]
fn brackets_and_suffixes_are_stripped_only_from_a_whole_link() {
    // A name that only looks like a link or an anchor is taken whole.
    for (name, note) in [
        ("[[Smith, John]]", "john.md"),
        ("\"C# tips\"", "C.md"),
        ("\"plan|draft\"", "plan.md"),
    ] {
        let f = Fixture::new();
        f.put(note, "quince\n");
        f.put("new.md", &format!("---\nsupersedes: {name}\n---\nlater\n"));
        assert_eq!(marked(&f), Vec::<String>::new(), "{name}");
    }
    for name in ["\"[[old-plan|the old one]]\"", "\"[[old-plan#why]]\""] {
        let f = Fixture::new();
        old_note(&f);
        f.put("new.md", &format!("---\nsupersedes: {name}\n---\nlater\n"));
        assert_eq!(marked(&f), ["plan/old-plan.md"], "{name}");
    }
    let f = Fixture::new();
    f.put("a#1.md", "quince\n");
    f.put("new.md", "---\nsupersedes: a#1\n---\nlater\n");
    assert_eq!(marked(&f), ["a#1.md"], "a name holding the mark");
}

#[test]
fn a_name_two_views_disagree_on_marks_nothing_in_either() {
    let f = Fixture::new();
    f.put("x/plan.md", "---\nvisibility: shared\n---\nquince\n");
    f.put("y/plan.md", "---\nvisibility: private\n---\nquince\n");
    f.put(
        "new.md",
        "---\nvisibility: shared\nsupersedes: plan\n---\nlater\n",
    );
    assert_eq!(marked(&f), Vec::<String>::new());
    let company = Query {
        scope: Scope::SharedOnly,
        rerank: false,
        ..query("quince")
    };
    let found = search(&f, &company);
    assert!(
        found.hits.iter().all(|h| h.overtaken_by.is_none()),
        "{:?}",
        paths(&found)
    );
}

/// `old.md` outranks `new.md`, which supersedes it.
fn outranked(f: &Fixture) {
    f.put("old.md", "quince quince quince\n");
    f.put(
        "new.md",
        "---\nsupersedes: old\n---\nquince and much else besides\n",
    );
}

#[test]
fn a_listed_overtaker_moves_above_the_note_it_replaces() {
    let f = Fixture::new();
    outranked(&f);
    let found = words(&f, "quince");
    assert_eq!(paths(&found), ["new.md", "old.md"]);
    assert!(!found.hits[0].added);
}

#[test]
fn an_overtaker_found_past_the_limit_was_still_judged() {
    let f = Fixture::new();
    outranked(&f);
    let one = search(
        &f,
        &Query {
            limit: 1,
            rerank: false,
            ..query("quince")
        },
    );
    assert_eq!(paths(&one), ["new.md"]);
    assert!(!one.hits[0].added);
}

#[test]
fn an_overtaker_already_above_keeps_its_place() {
    let f = Fixture::new();
    f.put(
        "new.md",
        "---\nsupersedes: old\n---\nquince quince quince quince\n",
    );
    f.put("mid.md", "quince quince in a jar\n");
    f.put(
        "old.md",
        "quince in a jar on a shelf in the pantry with the rest\n",
    );
    assert_eq!(paths(&words(&f, "quince")), ["new.md", "mid.md", "old.md"]);
}

#[test]
fn a_note_naming_itself_is_not_overtaken() {
    let f = Fixture::new();
    f.put("a.md", "---\nsupersedes: a\n---\nquince\n");
    assert_eq!(marked(&f), Vec::<String>::new());
}

#[test]
fn the_newest_of_two_overtakers_is_named() {
    let f = Fixture::new();
    f.put("a.md", "quince\n");
    f.put("b.md", "---\ndate: 2026-11-20\nsupersedes: a\n---\npear\n");
    f.put("c.md", "---\ndate: 2026-11-01\nsupersedes: a\n---\nplum\n");
    let found = words(&f, "quince");
    assert_eq!(hit(&found, "a.md").overtaken_by.as_deref(), Some("b.md"));
}

#[test]
fn an_earlier_dated_correction_still_marks() {
    let f = Fixture::new();
    old_note(&f);
    f.put(
        "fix.md",
        &format!("---\ndate: 2026-11-20\ncorrects: {OLD_ID}\n---\nlater\n"),
    );
    assert_eq!(marked(&f), ["plan/old-plan.md"]);
}

#[test]
fn a_framed_question_finds_its_topic() {
    let f = Fixture::new();
    f.put(
        "fees.md",
        "Omit pricing is a setup fee plus monthly care.\n",
    );
    let found = ask(&f, "what was the latest decision we made on omit pricing");
    assert!(!is_gap(&found));
    assert_eq!(paths(&found), ["fees.md"]);
}

#[test]
fn a_stripped_query_with_nothing_left_keeps_the_first_run() {
    let f = Fixture::new();
    f.put(
        "ship.md",
        "---\ntitle: Decision\n---\nThe latest decision: ship on Friday.\n",
    );
    let found = ask(&f, "latest decision on");
    assert_eq!(paths(&found), ["ship.md"]);
}

/// The jam notes: the framed question finds `plain` and `note`, and only the topic finds `sugar`,
/// which outscores both.
fn jam(f: &Fixture, date: &str) {
    let front = |title: &str| format!("---\ntitle: {title}\n{date}---\n");
    f.put(
        "plain.md",
        &format!(
            "{}Quince jam sets after forty minutes.\n",
            front("Quince jam")
        ),
    );
    f.put(
        "note.md",
        &format!(
            "{}A note on quince jam: use ripe fruit.\n",
            front("Quince jam note")
        ),
    );
}

#[test]
fn an_undated_topic_hit_sorts_by_score_among_the_first_runs() {
    let f = Fixture::new();
    jam(&f, "");
    f.put("sugar.md", "quince jam needs less sugar\n");
    let found = ask(&f, "latest note on quince jam");
    assert_eq!(paths(&found), ["sugar.md", "note.md", "plain.md"]);
}

#[test]
fn notes_of_one_day_keep_score_order() {
    let f = Fixture::new();
    jam(&f, "date: 2026-11-20\n");
    let found = ask(&f, "latest note on quince jam");
    assert_eq!(paths(&found), ["note.md", "plain.md"]);
}

/// A long note about many things: the framed question is under the floor against it, and short
/// notes that name the topic outrank it when the topic is asked alone, past what is judged.
#[test]
fn the_topic_is_also_judged_against_what_the_whole_question_found() {
    let f = Fixture::new();
    let filler = "The product keeps notes on the computer they were written on and searches them there. Each session starts by reading what is due and what the room holds. Rules are checked before a tool runs and every verdict is kept beside the call it judged. ".repeat(3);
    f.put(
        "direction.md",
        &format!(
            "---\ndate: 2026-11-24\n---\n{filler} Omit pricing: setup fee plus care. {filler} Most recent of all: a decision about the van.\n"
        ),
    );
    for n in 0..35 {
        f.put(
            &format!("page{n:02}.md"),
            "---\ndate: 2026-10-01\n---\nomit pricing page: the font is too small on a phone\n",
        );
    }
    let framed = ask(&f, "most recent decision about omit pricing");
    assert_eq!(paths(&framed)[0], "direction.md", "{:?}", paths(&framed));
}
