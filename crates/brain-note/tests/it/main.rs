mod gate;
mod vault;

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

fn fm_of<'s>(split: &'s mut Split<'_>) -> &'s mut brain_note::Frontmatter {
    split.fm.as_mut().unwrap().as_mut().unwrap()
}

#[test]
fn a_visibility_key_spelled_any_other_way_withholds() {
    for line in [
        "\"visibility\": shared",
        "'visibility': shared",
        "visibility : shared",
        "visibility\t: shared",
        "Visibility: shared",
        "{visibility: private}",
        "{title: t, visibility: private}",
        "? visibility\n: private",
    ] {
        let raw = format!("---\ntitle: t\n{line}\n---\nbody\n");
        assert_eq!(disc(&raw, Place::Kept), Disclosure::Private, "{line:?}");
    }
    let mention = "---\ntitle: About visibility in teams\n---\nbody\n";
    assert_eq!(disc(mention, Place::Kept), Disclosure::Shared);
}

#[test]
fn a_fence_with_trailing_space_is_still_frontmatter() {
    for raw in [
        "--- \nvisibility: private\n---\nbody\n",
        "---\t\r\nvisibility: private\r\n--- \r\nbody\r\n",
    ] {
        let split = Split::parse(raw);
        assert!(matches!(split.fm, Ok(Some(_))), "{raw:?}");
        assert_eq!(disclosure(&split, Place::Kept), Disclosure::Private);
        assert_eq!(split.render(), raw);
    }
}

#[test]
fn a_hash_without_a_space_is_part_of_the_value() {
    for v in ["shared#x", "public#"] {
        assert_eq!(
            disc(&with_visibility(v), Place::Kept),
            Disclosure::Private,
            "{v:?}"
        );
    }
    assert_eq!(
        disc(&with_visibility("shared\t# ok"), Place::Kept),
        Disclosure::Shared
    );
}

#[test]
fn block_scalars_and_mappings_read_as_other() {
    let raw =
        "---\ntitle: |\n  Multi\nvisibility:\n  level: shared\nkind: >\n  rule\n---\n# Heading\n";
    let split = Split::parse(raw);
    let fm = split.fm.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(fm.get("title"), Some(brain_note::Value::Other));
    assert_eq!(fm.get("visibility"), Some(brain_note::Value::Other));
    assert_eq!(disclosure(&split, Place::Kept), Disclosure::Private);
    let n = Note::parse(Path::new("n.md"), raw, Place::Kept);
    assert_eq!(n.title, "Heading");
    assert_eq!(n.kind, Kind::Knowledge);
}

/// A PEM armour line, built at run time so the repo's leak scan does not read the fixture as a key.
fn armour(edge: &str, label: &str) -> String {
    format!("-----{edge} {label} KEY-----")
}

#[test]
fn a_pem_block_split_by_a_blank_line_is_redacted() {
    let (begin, end) = (armour("BEGIN", "PRIVATE"), armour("END", "PRIVATE"));
    let body = format!("{begin}\nMIIEvQIBADANBgkqhkiG\n\nw0BAQEFAASCBKcwggSj\n{end}\n");
    let s = listed_summary(&body);
    assert!(!s.contains("BEGIN") && !s.contains("MIIEvQ"), "{s}");
}

#[test]
fn an_unterminated_pem_block_is_redacted() {
    let begin = armour("BEGIN", "RSA PRIVATE");
    let body = format!("Key below\n{begin}\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\nmore");
    let s = listed_summary(&body);
    assert!(!s.contains("MIIEvQ") && !s.contains("BEGIN"), "{s}");
    assert!(s.starts_with("Key below"), "{s}");
}

#[test]
fn a_crlf_or_blank_led_body_gives_its_first_paragraph() {
    assert_eq!(listed_summary("One line.\r\n\r\nSecond.\r\n"), "One line.");
    assert_eq!(listed_summary("\n\nFirst.\n\nSecond.\n"), "First.");
}

#[test]
fn ensure_id_refuses_a_conflicted_id_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("two.md");
    let raw = "---\nid: a\nid: b\n---\nx\n";
    fs::write(root.path().join(rel), raw).unwrap();
    assert_eq!(
        ensure_id(root.path(), rel, raw).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(
        brain_note::reassign_id(root.path(), rel, raw)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidData
    );
    assert_eq!(fs::read_to_string(root.path().join(rel)).unwrap(), raw);
}

#[test]
fn set_refuses_a_key_that_could_write_another() {
    let mut split = Split::parse("---\ntitle: x\n---\n");
    let fm = fm_of(&mut split);
    for key in ["a\nowner", "a: b", "a#", " a", "", "a b"] {
        assert_eq!(
            fm.set(key, "x"),
            Err(brain_note::FmError::Unsafe),
            "{key:?}"
        );
    }
    assert_eq!(split.render(), "---\ntitle: x\n---\n");
}

#[test]
fn set_replaces_a_key_with_its_continuation_lines() {
    let mut split = Split::parse("---\ntags:\n  - a\n  - b\ntitle: x\n---\n");
    fm_of(&mut split).set("tags", "c").unwrap();
    assert_eq!(split.render(), "---\ntags: c\ntitle: x\n---\n");
}

#[test]
fn a_set_value_reads_back_as_written() {
    for v in [
        "a:", "?x", ",x", "a\tb", "true", "null", "~", "123", "1.5", "no", "x #y", "-x",
    ] {
        let mut split = Split::parse("---\n---\n");
        fm_of(&mut split).set("title", v).unwrap();
        let rendered = split.render();
        let n = Note::parse(Path::new("n.md"), &rendered, Place::Kept);
        assert_eq!(n.title, v, "{v:?} written as {rendered:?}");
        assert!(
            rendered.contains(&format!("title: \"{v}\"")),
            "{v:?} written bare: {rendered:?}"
        );
    }
}

#[test]
fn set_keeps_crlf_line_ends() {
    let mut split = Split::parse("---\r\ntitle: x\r\n---\r\nbody\r\n");
    fm_of(&mut split).set("owner", "robin").unwrap();
    assert_eq!(
        split.render(),
        "---\r\ntitle: x\r\nowner: robin\r\n---\r\nbody\r\n"
    );
}

#[test]
fn a_new_block_takes_the_note_line_ends_and_keeps_its_bom() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("n.md");
    for (raw, eol) in [("a\r\nb\r\n", "\r\n"), ("\u{feff}a\nb\n", "\n")] {
        fs::write(root.path().join(rel), raw).unwrap();
        let (id, _) = ensure_id(root.path(), rel, raw).unwrap();
        let bom = if raw.starts_with('\u{feff}') {
            "\u{feff}"
        } else {
            ""
        };
        let body = raw.trim_start_matches('\u{feff}');
        assert_eq!(
            fs::read_to_string(root.path().join(rel)).unwrap(),
            format!("{bom}---{eol}id: {id}{eol}---{eol}{body}")
        );
    }
}

#[test]
fn reassign_id_changes_only_the_id() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("n.md");
    let raw = "---\nid: 0195f2e0-0000-7000-8000-000000000000\ntitle: x\n---\nbody\n";
    fs::write(root.path().join(rel), raw).unwrap();
    let id = brain_note::reassign_id(root.path(), rel, raw).unwrap();
    assert_eq!(id.get_version_num(), 7);
    assert_ne!(id.to_string(), "0195f2e0-0000-7000-8000-000000000000");
    assert_eq!(
        fs::read_to_string(root.path().join(rel)).unwrap(),
        format!("---\nid: {id}\ntitle: x\n---\nbody\n")
    );
}

#[test]
fn tags_read_from_block_and_inline_lists() {
    for raw in [
        "---\ntags: [a, \"b\"]\n---\n",
        "---\ntags:\n  - a\n  - 'b'\n---\n",
        "---\ntags:\n- a\n- b\n---\n",
    ] {
        assert_eq!(
            Note::parse(Path::new("n.md"), raw, Place::Kept).tags,
            vec!["a", "b"],
            "{raw:?}"
        );
    }
}

#[test]
fn a_file_ending_at_the_closing_fence_round_trips() {
    for raw in ["---\nid: x\n---", "---\r\nid: x\r\n---"] {
        let split = Split::parse(raw);
        assert!(matches!(split.fm, Ok(Some(_))));
        assert_eq!(split.render(), raw);
    }
}
