use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use brain_note::{
    Disclosure, Kind, Note, Place, Split, disclosure, ensure_id, listed_summary, rel_key, wikilinks,
};

fn disc(raw: &str, place: Place) -> Disclosure {
    disclosure(&Split::parse(raw), place)
}

fn with_visibility(v: &str) -> String {
    format!("---\ntitle: t\nvisibility: {v}\n---\nbody\n")
}

#[test]
fn absent_visibility_is_shared() {
    assert_eq!(
        disc("---\ntitle: t\n---\nbody\n", Place::Kept),
        Disclosure::Shared
    );
    assert_eq!(
        disc("no frontmatter at all\n", Place::Kept),
        Disclosure::Shared
    );
}

#[test]
fn explicit_shared_and_public_are_shared() {
    assert_eq!(
        disc(&with_visibility("shared"), Place::Kept),
        Disclosure::Shared
    );
    assert_eq!(
        disc(&with_visibility("public"), Place::Kept),
        Disclosure::Shared
    );
}

#[test]
fn listed_is_its_own_disclosure() {
    assert_eq!(
        disc(&with_visibility("listed"), Place::Kept),
        Disclosure::Listed
    );
}

#[test]
fn explicit_private_is_private() {
    assert_eq!(
        disc(&with_visibility("private"), Place::Kept),
        Disclosure::Private
    );
}

#[test]
fn private_folder_wins_over_frontmatter() {
    assert_eq!(
        disc(&with_visibility("shared"), Place::Private),
        Disclosure::Private
    );
}

#[test]
fn an_inbox_note_is_never_shared() {
    assert_eq!(
        disc(&with_visibility("shared"), Place::Inbox),
        Disclosure::Private
    );
    assert_eq!(disc("plain\n", Place::Inbox), Disclosure::Private);
}

#[test]
fn an_unparseable_value_withholds_rather_than_falling_through() {
    for v in [
        "pubic",
        "",
        "yes",
        "true",
        "[private]",
        "[shared]",
        "publi c",
        "\"public",
        "\"private#1\"",
    ] {
        assert_eq!(
            disc(&with_visibility(v), Place::Kept),
            Disclosure::Private,
            "value {v:?}"
        );
    }
}

#[test]
fn quotes_and_trailing_comments_are_normalised() {
    for v in ["'shared'", "\"shared\"", "shared  # ok", "SHARED"] {
        assert_eq!(
            disc(&with_visibility(v), Place::Kept),
            Disclosure::Shared,
            "value {v:?}"
        );
    }
}

#[test]
fn an_indented_visibility_line_withholds() {
    let nested = "---\nmeta:\n  visibility: private\n---\nbody\n";
    let tabbed = "---\ntitle: t\n\tvisibility: private\n---\nbody\n";
    assert_eq!(disc(nested, Place::Kept), Disclosure::Private);
    assert_eq!(disc(tabbed, Place::Kept), Disclosure::Private);
}

#[test]
fn a_duplicate_visibility_key_withholds() {
    let raw = "---\nvisibility: shared\nvisibility: shared\n---\nbody\n";
    assert_eq!(disc(raw, Place::Kept), Disclosure::Private);
}

#[test]
fn broken_frontmatter_withholds() {
    let raw = "---\nvisibility: shared\ntitle: never closed\nbody text\n";
    let split = Split::parse(raw);
    assert!(split.fm.is_err());
    assert_eq!(split.body, raw);
    assert_eq!(disclosure(&split, Place::Kept), Disclosure::Private);
}

#[test]
fn the_listed_summary_is_scrubbed_and_capped() {
    let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
    let pat = "ghp_abcdefghijklmnopqrstuvwxyz0123456789AB";
    let s = listed_summary(&format!("Use {key} and {pat} here.\n\nSecond paragraph."));
    assert_eq!(s, "Use [REDACTED] and [REDACTED] here.");

    let long = "word ".repeat(100);
    let cut = listed_summary(&long);
    assert!(cut.chars().count() <= 303, "{} chars", cut.chars().count());
    assert!(cut.ends_with("..."));
}

#[test]
fn a_secret_cut_by_the_cap_is_still_redacted() {
    let base64 = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7".repeat(8);
    let pem = format!("-----BEGIN PRIVATE KEY-----\n{base64}\n-----END PRIVATE KEY-----");
    let body = format!("{} {pem} tail", "lead ".repeat(50));
    let s = listed_summary(&body);
    assert!(!s.contains("MIIEvQIBADAN"), "key material leaked: {s}");
}

#[test]
fn crlf_and_bom_round_trip_byte_for_byte() {
    for raw in [
        "\u{feff}---\r\ntitle: x\r\n# a comment\r\ntags: [a, b]\r\n---\r\nbody\r\n",
        "---\ntitle: x\n---\n",
        "just a body\n",
    ] {
        assert_eq!(Split::parse(raw).render(), raw);
    }
}

#[test]
fn set_changes_only_that_key() {
    let raw = "---\n# keep me\ntitle: x\ntags:\n  - a\nid: old\n---\nbody\n";
    let mut split = Split::parse(raw);
    split
        .fm
        .as_mut()
        .unwrap()
        .as_mut()
        .unwrap()
        .set("id", "new")
        .unwrap();
    assert_eq!(
        split.render(),
        "---\n# keep me\ntitle: x\ntags:\n  - a\nid: new\n---\nbody\n"
    );
    split
        .fm
        .as_mut()
        .unwrap()
        .as_mut()
        .unwrap()
        .set("owner", "robin")
        .unwrap();
    assert_eq!(
        split.render(),
        "---\n# keep me\ntitle: x\ntags:\n  - a\nid: new\nowner: robin\n---\nbody\n"
    );
}

#[test]
fn a_value_with_a_newline_cannot_inject_a_key() {
    let mut split = Split::parse("---\ntitle: x\n---\n");
    let fm = split.fm.as_mut().unwrap().as_mut().unwrap();
    assert!(fm.set("title", "x\nowner: eve").is_err());
    assert!(fm.set("title", "x\rowner: eve").is_err());
    assert_eq!(split.render(), "---\ntitle: x\n---\n");
}

#[test]
fn metadata_type_is_read_by_path() {
    let raw = "---\nname: n\nmetadata:\n  node_type: memory\n  type: feedback\n---\nbody\n";
    let split = Split::parse(raw);
    let fm = split.fm.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(
        fm.get_path(&["metadata", "type"]),
        Some(brain_note::Value::Text("feedback".into()))
    );
}

#[test]
fn ensure_id_writes_a_v7_only_when_missing() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("n.md");
    fs::write(root.path().join(rel), "---\ntitle: x\n---\nbody\n").unwrap();
    let raw = fs::read_to_string(root.path().join(rel)).unwrap();
    let (id, wrote) = ensure_id(root.path(), rel, &raw).unwrap();
    assert!(wrote);
    assert_eq!(id.get_version_num(), 7);
    let after = fs::read_to_string(root.path().join(rel)).unwrap();
    assert_eq!(after, format!("---\ntitle: x\nid: {id}\n---\nbody\n"));
    let (again, wrote) = ensure_id(root.path(), rel, &after).unwrap();
    assert_eq!((again, wrote), (id, false));
    assert_eq!(fs::read_to_string(root.path().join(rel)).unwrap(), after);
}

#[test]
fn an_id_is_never_written_into_broken_frontmatter() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("broken.md");
    let raw = "---\nvisibility: private\nnever closed\n";
    fs::write(root.path().join(rel), raw).unwrap();
    let err = ensure_id(root.path(), rel, raw).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert_eq!(fs::read_to_string(root.path().join(rel)).unwrap(), raw);
    assert_eq!(disc(raw, Place::Kept), Disclosure::Private);
}

#[test]
fn rel_keys_use_forward_slashes() {
    assert_eq!(rel_key(&Path::new("a").join("b").join("c.md")), "a/b/c.md");
}

#[test]
fn wikilinks_follow_learning_loop() {
    assert_eq!(
        wikilinks("see [[A|b]] and [[a#x]] and [[ C ]] and [[]]"),
        vec!["a", "c"]
    );
}

#[test]
fn title_falls_back_to_heading_then_file_stem() {
    let rel = Path::new("dir/my-note.md");
    assert_eq!(
        Note::parse(rel, "---\ntitle: \"Given\"\n---\n# Heading\n", Place::Kept).title,
        "Given"
    );
    assert_eq!(
        Note::parse(rel, "intro\n# Heading\nmore\n", Place::Kept).title,
        "Heading"
    );
    assert_eq!(
        Note::parse(rel, "no heading\n", Place::Kept).title,
        "my-note"
    );
}

#[test]
fn a_note_reads_its_fields() {
    let raw = "---\nid: 0195f2e0-0000-7000-8000-000000000000\ntags: [a, b]\nkind: rule\nowner: robin\nreview: 2027-02-22\n---\nbody [[x]]\n";
    let n = Note::parse(Path::new("w/r.md"), raw, Place::Kept);
    assert_eq!(
        n.id.unwrap().to_string(),
        "0195f2e0-0000-7000-8000-000000000000"
    );
    assert_eq!(n.tags, vec!["a", "b"]);
    assert_eq!(n.kind, Kind::Rule);
    assert_eq!(n.owner.as_deref(), Some("robin"));
    assert_eq!(n.review, Some(jiff::civil::date(2027, 2, 22)));
    assert_eq!(n.key, "w/r.md");
    assert_eq!(n.links, vec!["x"]);
}
