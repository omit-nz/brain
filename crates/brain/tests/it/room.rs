//! Rooms: `brain room post`, `new` and `list` over `rooms/<name>/` in the vault, with the seen
//! set in brain's data folder.

use std::fs;
use std::path::Path;

use super::Home;

const T: &str = "2026-11-26T08:00:00Z";

fn home() -> Home {
    let h = Home::new();
    h.init();
    h
}

/// `brain --now <at> room <args>`, which must succeed; its stdout.
fn room(h: &Home, at: &str, args: &[&str]) -> String {
    let mut all = vec!["--now", at, "room"];
    all.extend_from_slice(args);
    h.run(&all)
}

/// `brain room <args>` expected to fail; its stderr.
fn refused(h: &Home, args: &[&str]) -> String {
    let mut all = vec!["--now", T, "room"];
    all.extend_from_slice(args);
    let out = h.brain().args(&all).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "brain {all:?} should fail");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn read(h: &Home, rel: &str) -> String {
    fs::read_to_string(h.vault.path().join(rel)).unwrap()
}

/// Every file under `dir`, as paths relative to it.
fn files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if dir.is_dir() {
        for e in walk(dir) {
            out.push(
                e.strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    out.sort();
    out
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn a_post_lands_in_its_room_named_by_time_and_title() {
    let h = home();
    let out = room(
        &h,
        T,
        &["post", "Ship it", "--title", "Release go", "--as", "chief"],
    );
    assert_eq!(out.trim(), "rooms/squads/20261126T080000Z-release-go.md");
    assert!(
        h.vault
            .path()
            .join("rooms/squads/20261126T080000Z-release-go.md")
            .is_file()
    );
}

#[test]
fn a_post_has_the_frontmatter_of_a_message() {
    let h = home();
    let rel = room(
        &h,
        T,
        &["post", "Ship it", "--title", "Release go", "--as", "chief"],
    );
    let text = read(&h, rel.trim());
    let (fm, body) = text
        .strip_prefix("---\n")
        .and_then(|t| t.split_once("---\n"))
        .unwrap();
    let lines: Vec<&str> = fm.lines().collect();
    let id = lines[0].strip_prefix("id: ").unwrap();
    assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 7);
    assert_eq!(
        &lines[1..],
        [
            "room: squads",
            "author: chief",
            "posted: 2026-11-26T08:00:00Z",
            "title: Release go",
            "visibility: private",
        ]
    );
    assert_eq!(body, "Ship it\n");

    // No title: the text's first line; no --as: the config's me.
    let rel = room(&h, T, &["post", "\n  First line here  \nsecond line"]);
    assert_eq!(
        rel.trim(),
        "rooms/squads/20261126T080000Z-first-line-here.md"
    );
    let text = read(&h, rel.trim());
    assert!(text.contains("\nauthor: robin\n"), "{text}");
    assert!(text.contains("\ntitle: First line here\n"), "{text}");
    assert!(
        !text.contains("owner:") && !text.contains("review:"),
        "{text}"
    );
}

#[test]
fn a_long_first_line_is_cut_at_80_characters_for_the_title() {
    let h = home();
    let long = "é".repeat(100);
    let rel = room(&h, T, &["post", &long]);
    let text = read(&h, rel.trim());
    assert!(
        text.contains(&format!("\ntitle: {}\n", "é".repeat(80))),
        "{text}"
    );
}

#[test]
fn two_posts_in_one_second_with_one_title_keep_both() {
    let h = home();
    let a = room(&h, T, &["post", "first", "--title", "Release go"]);
    let b = room(&h, T, &["post", "second", "--title", "Release go"]);
    assert_eq!(a.trim(), "rooms/squads/20261126T080000Z-release-go.md");
    assert_eq!(b.trim(), "rooms/squads/20261126T080000Z-release-go-1.md");
    assert!(read(&h, a.trim()).ends_with("first\n"));
    assert!(read(&h, b.trim()).ends_with("second\n"));
}

#[test]
fn a_room_name_outside_the_pattern_is_refused_and_nothing_is_written() {
    let h = home();
    let before = files(h.vault.path());
    let long = "a".repeat(41);
    for name in [
        "../x",
        "Squads",
        "a/b",
        "",
        "-a",
        "a_b",
        "a.b",
        long.as_str(),
    ] {
        let err = refused(&h, &["post", "hi", &format!("--room={name}")]);
        assert!(err.contains("room name"), "{name}: {err}");
    }
    assert_eq!(files(h.vault.path()), before);
    assert!(!h.vault.path().join("rooms").exists());
    // 40 characters, digits and inner dashes are a name.
    let ok = format!("a1-{}", "b".repeat(37));
    room(&h, T, &["post", "hi", "--room", &ok]);
    assert!(h.vault.path().join("rooms").join(&ok).is_dir());
}

#[test]
fn a_title_with_path_characters_stays_in_the_room() {
    let h = home();
    for title in ["../../../etc/passwd", "a/b\\c", ".."] {
        let rel = room(&h, T, &["post", "x", "--title", title]);
        let rel = rel.trim();
        let name = rel.strip_prefix("rooms/squads/").unwrap();
        assert!(!name.contains('/') && !name.contains('\\'), "{rel}");
        assert!(h.vault.path().join(rel).is_file());
    }
    let err = refused(&h, &["post", "x", "--title", "two\nlines"]);
    assert!(err.contains("line break"), "{err}");
}

#[test]
fn an_author_name_that_is_empty_long_or_two_lines_is_refused() {
    let h = home();
    let long = "n".repeat(61);
    for name in ["", long.as_str(), "a\nb", "a\rb"] {
        let err = refused(&h, &["post", "x", &format!("--as={name}")]);
        assert!(err.contains("name"), "{name:?}: {err}");
        let err = refused(&h, &["new", &format!("--as={name}")]);
        assert!(err.contains("name"), "{name:?}: {err}");
    }
    room(&h, T, &["post", "x", "--as", &"n".repeat(60)]);
}

#[test]
fn an_empty_post_is_refused() {
    let h = home();
    let err = refused(&h, &["post", "  \n "]);
    assert!(err.contains("empty"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_room_folder_that_is_a_link_is_refused() {
    let h = home();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::create_dir_all(h.vault.path().join("rooms")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), h.vault.path().join("rooms/squads")).unwrap();
    let err = refused(&h, &["post", "x"]);
    assert!(err.contains("link"), "{err}");
    let err = refused(&h, &["new"]);
    assert!(err.contains("link"), "{err}");
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);

    let h = home();
    std::os::unix::fs::symlink(elsewhere.path(), h.vault.path().join("rooms")).unwrap();
    let err = refused(&h, &["post", "x"]);
    assert!(err.contains("link"), "{err}");
    let err = refused(&h, &["list"]);
    assert!(err.contains("link"), "{err}");
    assert_eq!(fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

/// The post titles `brain room new` printed, in order.
fn titles(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| !l.starts_with("    ") && l.contains("  ") && !l.contains("(unreadable: "))
        .map(|l| l.rsplit("  ").next().unwrap().to_string())
        .collect()
}

#[test]
fn new_shows_each_post_once_per_reader() {
    let h = home();
    room(&h, T, &["post", "a body", "--title", "A", "--as", "chief"]);
    room(
        &h,
        "2026-11-26T08:00:01Z",
        &["post", "b body", "--title", "B"],
    );
    let out = room(&h, T, &["new", "--as", "chief"]);
    assert_eq!(titles(&out), ["A", "B"], "{out}");
    assert!(
        out.contains("2026-11-26T08:00:00Z  chief  A\n    a body\n"),
        "{out}"
    );
    assert!(out.contains("2026-11-26T08:00:01Z  robin  B\n"), "{out}");
    assert_eq!(
        room(&h, T, &["new", "--as", "chief"]).trim(),
        "nothing new in squads"
    );
    room(&h, "2026-11-26T08:00:02Z", &["post", "c", "--title", "C"]);
    assert_eq!(titles(&room(&h, T, &["new", "--as", "chief"])), ["C"]);
    assert_eq!(
        titles(&room(&h, T, &["new", "--as", "coo"])),
        ["A", "B", "C"]
    );
    // Without --as the reader is the config's me, a reader of its own.
    assert_eq!(titles(&room(&h, T, &["new"])), ["A", "B", "C"]);
    // Another room is read apart.
    assert_eq!(
        room(&h, T, &["new", "--room", "ops", "--as", "chief"]).trim(),
        "nothing new in ops"
    );
}

#[test]
fn a_post_written_late_in_a_seen_second_is_still_new() {
    let h = home();
    room(&h, T, &["post", "z", "--title", "zeta"]);
    assert_eq!(titles(&room(&h, T, &["new", "--as", "chief"])), ["zeta"]);
    room(&h, T, &["post", "a", "--title", "alpha"]);
    assert_eq!(titles(&room(&h, T, &["new", "--as", "chief"])), ["alpha"]);
}

/// A post file put straight into the room, as another writer would leave it.
fn put_post(h: &Home, name: &str, title: &str) {
    h.put(
        &format!("rooms/squads/{name}"),
        &format!(
            "---\nid: 0190b7c2-0000-7000-8000-000000000000\nroom: squads\nauthor: lane\nposted: 2026-11-26T08:00:00Z\ntitle: {title}\nvisibility: private\n---\nbody of {title}\n"
        ),
    );
}

#[test]
fn fifty_one_new_posts_come_in_two_pages() {
    let h = home();
    for i in 0..51 {
        put_post(
            &h,
            &format!("20261126T0800{i:02}Z-p.md"),
            &format!("p{i:02}"),
        );
    }
    let out = room(&h, T, &["new"]);
    let want: Vec<String> = (0..50).map(|i| format!("p{i:02}")).collect();
    assert_eq!(titles(&out), want);
    assert!(
        out.trim_end()
            .ends_with("1 more not shown; run brain room new again"),
        "{out}"
    );
    let out = room(&h, T, &["new"]);
    assert_eq!(titles(&out), ["p50"]);
    assert!(!out.contains("more not shown"), "{out}");
}

#[test]
fn only_finished_posts_are_read() {
    let h = home();
    put_post(&h, "20261126T080000Z-real.md", "real");
    put_post(&h, ".brain-x1y2z3", "temp");
    put_post(&h, ".20261126T080001Z-hidden.md", "hidden");
    h.put("rooms/squads/notes.txt", "not a post\n");
    put_post(&h, "sub/20261126T080002Z-deep.md", "deep");
    assert_eq!(titles(&room(&h, T, &["new"])), ["real"]);
    assert!(room(&h, T, &["list"]).contains("squads  1 post  "));
}

#[test]
fn an_unreadable_post_is_shown_by_name_and_the_rest_still_read() {
    let h = home();
    put_post(&h, "20261126T080000Z-a.md", "a");
    let bad = h.vault.path().join("rooms/squads/20261126T080001Z-bad.md");
    fs::write(&bad, b"---\ntitle: \xff\xfe\n---\n").unwrap();
    put_post(&h, "20261126T080002Z-c.md", "c");
    let out = room(&h, T, &["new"]);
    assert_eq!(titles(&out), ["a", "c"], "{out}");
    assert!(
        out.contains("20261126T080001Z-bad.md  (unreadable: "),
        "{out}"
    );
    assert_eq!(room(&h, T, &["new"]).trim(), "nothing new in squads");
}

#[test]
fn posts_stay_new_when_the_seen_set_cannot_be_written() {
    let h = home();
    room(&h, T, &["post", "x", "--title", "kept"]);
    let seen = h.home.path().join("data/rooms");
    fs::create_dir_all(seen.parent().unwrap()).unwrap();
    fs::write(&seen, "in the way").unwrap();
    refused(&h, &["new"]);
    fs::remove_file(&seen).unwrap();
    assert_eq!(titles(&room(&h, T, &["new"])), ["kept"]);
}

#[test]
fn list_shows_each_room_with_its_count_and_newest() {
    let h = home();
    assert_eq!(room(&h, T, &["list"]).trim(), "no rooms yet");
    room(&h, T, &["post", "a"]);
    room(&h, "2026-11-26T08:00:01Z", &["post", "b"]);
    room(&h, "2026-11-27T09:30:00Z", &["post", "c", "--room", "ops"]);
    fs::create_dir_all(h.vault.path().join("rooms/empty")).unwrap();
    fs::create_dir_all(h.vault.path().join("rooms/Not-A-Room")).unwrap();
    assert_eq!(
        room(&h, T, &["list"]).lines().collect::<Vec<_>>(),
        [
            "empty  0 posts",
            "ops  1 post  newest 2026-11-27T09:30:00Z",
            "squads  2 posts  newest 2026-11-26T08:00:01Z",
        ]
    );
}

#[test]
fn a_post_is_found_by_search_but_never_shared() {
    let h = home();
    let rel = room(
        &h,
        T,
        &["post", "the zanzibar deploy is green", "--title", "Deploy"],
    );
    h.run(&["index"]);
    let mine = h.json(&["search", "--json", "zanzibar"]);
    let paths: Vec<&str> = mine["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, [rel.trim()]);
    let shared = h.json(&["search", "--json", "--shared-only", "zanzibar"]);
    assert_eq!(shared["hits"].as_array().unwrap().len(), 0, "{shared}");
}

#[test]
fn a_title_that_needs_quotes_reads_back_as_written() {
    let h = home();
    let title = r#"say "hi": now \ then"#;
    room(&h, T, &["post", "x", "--title", title]);
    assert_eq!(titles(&room(&h, T, &["new"])), [title]);
}
