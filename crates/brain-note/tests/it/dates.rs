use std::path::Path;

use brain_note::{Note, Place};

/// The note's candidate dates, in the order they are tried.
fn dates(path: &str, raw: &str) -> Vec<String> {
    Note::parse(Path::new(path), raw, Place::Kept)
        .dates
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn a_notes_date_comes_from_frontmatter_name_or_first_line() {
    assert_eq!(
        dates("a.md", "---\ndate: 2026-10-27\n---\nbody\n"),
        ["2026-10-27"]
    );
    assert_eq!(
        dates("a.md", "---\ncreated: 2026-10-27T09:00:00\n---\nbody\n"),
        ["2026-10-27"]
    );
    assert_eq!(dates("notes_2026_11_27.md", "body\n"), ["2026-11-27"]);
    assert_eq!(dates("call-2026-11-13-x.md", "body\n"), ["2026-11-13"]);
    assert_eq!(
        dates(
            "a.md",
            "---\nid: x\n---\n\n2026-11-24 rulings: a fee\nlater 2026-01-01\n"
        ),
        ["2026-11-24"]
    );
    assert_eq!(
        dates("a.md", "Priority rulings 27 Nov 2026: absum\n"),
        ["2026-11-27"]
    );
    assert_eq!(dates("a.md", "on 3 September 2026, said\n"), ["2026-09-03"]);
    assert_eq!(
        dates(
            "2026-11-01-x.md",
            "---\ndate: 2026-11-05\n---\n2026-11-02 seen\n"
        ),
        ["2026-11-05", "2026-11-01", "2026-11-02"]
    );
    // A bulk edit stamped a later date: the name's goes first.
    assert_eq!(
        dates("2026-05-01-x.md", "---\ndate: 2026-08-02\n---\nbody\n"),
        ["2026-05-01", "2026-08-02"]
    );
    for none in [
        "body\n2026-11-27 on the second line\n",
        "v1.2026-13-40 is no date\n",
        "id 12026-11-27x\n",
        "---\ndate: soon\n---\nbody\n",
        "5 Maybe 2026\n",
    ] {
        assert_eq!(dates("a.md", none), Vec::<String>::new(), "{none:?}");
    }
}

#[test]
fn corrects_and_supersedes_are_kept_as_written() {
    let parse = |raw: &str| Note::parse(Path::new("a.md"), raw, Place::Kept);
    let n = parse("---\ncorrects: 0195f2e0\nsupersedes: old-plan.md\n---\nx\n");
    assert_eq!(n.corrects.as_deref(), Some("0195f2e0"));
    assert_eq!(n.supersedes.as_deref(), Some("old-plan.md"));
    let n = parse("---\nsupersedes:\n  - one\n  - two\n---\nx\n");
    assert_eq!(n.supersedes.as_deref(), Some("one\ntwo"));
    assert_eq!(parse("x\n").supersedes, None);
}
