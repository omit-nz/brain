use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::Home;

const NOW: &str = "2026-11-25T00:00:00Z";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dst);
        } else {
            fs::copy(e.path(), dst).unwrap();
        }
    }
}

/// Every file under `root` with its bytes, keyed by its path with `/`.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel: Vec<_> = p
                    .strip_prefix(root)
                    .unwrap()
                    .iter()
                    .map(|c| c.to_string_lossy().to_string())
                    .collect();
                out.insert(rel.join("/"), fs::read(&p).unwrap());
            }
        }
    }
    out
}

struct Import {
    h: Home,
    src: tempfile::TempDir,
    mem: tempfile::TempDir,
}

impl Import {
    fn new() -> Import {
        let i = Import {
            h: Home::new(),
            src: tempfile::tempdir().unwrap(),
            mem: tempfile::tempdir().unwrap(),
        };
        copy_tree(&fixture("ll-vault"), i.src.path());
        copy_tree(&fixture("ll-memory"), i.mem.path());
        i.h.init();
        i
    }

    fn src(&self) -> &str {
        self.src.path().to_str().unwrap()
    }

    fn import(&self, extra: &[&str]) -> String {
        let mut args = vec![
            "--now",
            NOW,
            "import",
            "learning-loop",
            "--from",
            self.src(),
        ];
        args.extend_from_slice(extra);
        self.h.run(&args)
    }

    fn copy(&self, rel: &str) -> String {
        fs::read_to_string(self.h.vault.path().join(rel)).unwrap()
    }

    fn source(&self, rel: &str) -> String {
        fs::read_to_string(self.src.path().join(rel)).unwrap()
    }
}

fn line<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}: ")))
}

#[test]
fn an_undeclared_note_imports_private() {
    let i = Import::new();
    i.import(&[]);
    assert_eq!(
        line(&i.copy("3-permanent/kept-idea.md"), "visibility"),
        Some("private")
    );
}

#[test]
fn a_shared_folder_imports_shared() {
    let i = Import::new();
    i.import(&["--share", "3-permanent", "--no-ll-config"]);
    assert_eq!(
        line(&i.copy("3-permanent/kept-idea.md"), "visibility"),
        Some("shared")
    );
    assert_eq!(
        line(&i.copy("3-permanent/separation-agreement.md"), "visibility"),
        Some("shared")
    );
    assert_eq!(
        line(&i.copy("2-literature/literature-note.md"), "visibility"),
        Some("private")
    );
}

#[test]
fn a_blocklisted_note_stays_private_under_a_shared_folder() {
    let i = Import::new();
    let config = fixture("ll-config");
    i.import(&[
        "--share",
        "3-permanent",
        "--ll-config",
        config.to_str().unwrap(),
    ]);
    assert_eq!(
        line(&i.copy("3-permanent/separation-agreement.md"), "visibility"),
        Some("private")
    );
    assert_eq!(
        line(&i.copy("3-permanent/kept-idea.md"), "visibility"),
        Some("shared")
    );
}

#[test]
fn share_without_a_blocklist_choice_is_refused() {
    let i = Import::new();
    let before = snapshot(i.h.vault.path());
    let out =
        i.h.brain()
            .args([
                "import",
                "learning-loop",
                "--from",
                i.src(),
                "--share",
                "3-permanent",
            ])
            .output()
            .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(snapshot(i.h.vault.path()), before);
}

#[test]
fn declared_visibility_is_kept() {
    let i = Import::new();
    i.import(&["--share", "3-permanent", "--no-ll-config"]);
    assert_eq!(
        i.copy("3-permanent/declared.md"),
        i.source("3-permanent/declared.md")
    );
}

#[test]
fn ids_tags_links_and_intentions_come_across_as_they_are() {
    let i = Import::new();
    i.import(&[]);
    let copy = i.copy("3-permanent/kept-idea.md");
    for key in ["visibility", "owner", "review"] {
        assert!(line(&copy, key).is_some(), "{key} missing:\n{copy}");
    }
    let stripped: Vec<&str> = copy
        .lines()
        .filter(|l| {
            !["visibility: ", "owner: ", "review: "]
                .iter()
                .any(|k| l.starts_with(k))
        })
        .collect();
    assert_eq!(
        stripped,
        i.source("3-permanent/kept-idea.md")
            .lines()
            .collect::<Vec<_>>()
    );
    let inbox = i.copy("0-inbox/captured.md");
    assert_eq!(line(&inbox, "owner"), None, "an inbox note gets no owner");
    assert_eq!(line(&inbox, "review"), None);
}

#[test]
fn imported_kept_notes_get_owner_and_review() {
    let i = Import::new();
    for n in 0..1000 {
        let id = format!("0195f2e0-0000-7000-8000-{n:012x}");
        fs::write(
            i.src.path().join(format!("3-permanent/n{n:04}.md")),
            format!("---\nid: {id}\n---\nnote {n}\n"),
        )
        .unwrap();
    }
    i.import(&[]);
    let today = jiff::civil::date(2026, 11, 25);
    let mut days = std::collections::BTreeSet::new();
    for n in 0..1000 {
        let copy = i.copy(&format!("3-permanent/n{n:04}.md"));
        assert_eq!(line(&copy, "owner"), Some("robin"));
        let review: jiff::civil::Date = line(&copy, "review").unwrap().parse().unwrap();
        let ahead = (review - today).get_days();
        assert!(
            (1..=90).contains(&ahead),
            "review {review} is {ahead} days out"
        );
        days.insert(review);
    }
    assert!(days.len() >= 80, "only {} distinct review days", days.len());
}

#[test]
fn import_never_writes_the_source_vault() {
    let i = Import::new();
    let before = snapshot(i.src.path());
    let mem_before = snapshot(i.mem.path());
    let mem = i.mem.path().to_str().unwrap().to_string();
    i.import(&["--memory", &mem, "--share", "3-permanent", "--no-ll-config"]);
    i.h.run(&["index"]);
    i.import(&["--memory", &mem, "--share", "3-permanent", "--no-ll-config"]);
    assert_eq!(snapshot(i.src.path()), before);
    assert_eq!(snapshot(i.mem.path()), mem_before);
}

#[test]
fn importing_a_vault_into_itself_is_refused() {
    let i = Import::new();
    let own = i.h.vault.path().to_str().unwrap().to_string();
    let out =
        i.h.brain()
            .args(["import", "learning-loop", "--from", &own])
            .output()
            .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn reimport_is_idempotent() {
    let i = Import::new();
    i.import(&[]);
    let after_first = snapshot(i.h.vault.path());
    let out = i.import(&[]);
    assert!(out.contains("0 new, 0 updated"), "{out}");
    assert_eq!(snapshot(i.h.vault.path()), after_first);
}

#[test]
fn reimporting_memory_is_idempotent() {
    let i = Import::new();
    let mem = i.mem.path().to_str().unwrap().to_string();
    i.import(&["--memory", &mem]);
    let after_first = snapshot(i.h.vault.path());
    let out = i.import(&["--memory", &mem]);
    assert!(out.contains("0 new, 0 updated"), "{out}");
    assert_eq!(snapshot(i.h.vault.path()), after_first);
}

#[test]
fn a_source_change_updates_an_untouched_copy() {
    let i = Import::new();
    i.import(&[]);
    let id = line(&i.copy("3-permanent/kept-idea.md"), "id")
        .unwrap()
        .to_string();
    let src = i
        .source("3-permanent/kept-idea.md")
        .replace("A kept idea", "A revised idea");
    fs::write(i.src.path().join("3-permanent/kept-idea.md"), src).unwrap();
    let out = i.import(&[]);
    assert!(out.contains("0 new, 1 updated"), "{out}");
    let copy = i.copy("3-permanent/kept-idea.md");
    assert!(copy.contains("A revised idea"));
    assert_eq!(line(&copy, "id"), Some(id.as_str()));
}

#[test]
fn an_updated_note_without_an_id_keeps_its_copy_id() {
    let i = Import::new();
    fs::write(
        i.src.path().join("3-permanent/plain.md"),
        "no frontmatter, first version\n",
    )
    .unwrap();
    i.import(&[]);
    let id = line(&i.copy("3-permanent/plain.md"), "id")
        .unwrap()
        .to_string();
    fs::write(
        i.src.path().join("3-permanent/plain.md"),
        "no frontmatter, second version\n",
    )
    .unwrap();
    let out = i.import(&[]);
    assert!(out.contains("0 new, 1 updated"), "{out}");
    let copy = i.copy("3-permanent/plain.md");
    assert!(copy.contains("second version"));
    assert_eq!(line(&copy, "id"), Some(id.as_str()));
    let again = i.import(&[]);
    assert!(again.contains("0 new, 0 updated"), "{again}");
}

/// With another writer holding the lock the import copies and indexes nothing, so each copy must
/// carry its id from the import itself, and the ledger must record those final bytes.
#[test]
fn an_import_beside_a_running_writer_writes_ids_and_leaves_the_index_alone() {
    let i = Import::new();
    fs::write(
        i.src.path().join("3-permanent/plain.md"),
        "no frontmatter\n",
    )
    .unwrap();
    fs::write(i.src.path().join("0-inbox/plain.md"), "an inbox note\n").unwrap();
    let data = i.h.home.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let _writer = brain_platform::fs::Lock::acquire(&data.join("writer.lock")).unwrap();
    let out = i.import(&[]);
    assert!(out.contains("brain's daemon will index them"), "{out}");
    assert!(
        out.contains("with no daemon running, run `brain index`"),
        "the writer here is no daemon: {out}"
    );
    for rel in ["3-permanent/plain.md", "0-inbox/plain.md"] {
        let copy = i.copy(rel);
        let id = line(&copy, "id").unwrap_or_else(|| panic!("{rel} has no id:\n{copy}"));
        uuid::Uuid::parse_str(id).unwrap();
    }
    assert_eq!(i.h.json(&["status", "--json"])["notes"], 0);
    let again = i.import(&[]);
    assert!(again.contains("0 new, 0 updated"), "{again}");
    assert!(again.contains("0 conflicts"), "{again}");
}

#[test]
fn a_note_changed_on_both_sides_is_reported_not_overwritten() {
    let i = Import::new();
    i.import(&[]);
    let dst = i.h.vault.path().join("3-permanent/kept-idea.md");
    let mine = i
        .copy("3-permanent/kept-idea.md")
        .replace("A kept idea", "My edit");
    fs::write(&dst, &mine).unwrap();
    let src = i
        .source("3-permanent/kept-idea.md")
        .replace("A kept idea", "Their edit");
    fs::write(i.src.path().join("3-permanent/kept-idea.md"), src).unwrap();
    let out = i.import(&[]);
    assert!(out.contains("1 conflict"), "{out}");
    assert!(out.contains("3-permanent/kept-idea.md"), "{out}");
    assert_eq!(fs::read_to_string(&dst).unwrap(), mine);
}

#[test]
fn both_memory_shapes_import_with_the_right_kind() {
    let i = Import::new();
    let mem = i.mem.path().to_str().unwrap().to_string();
    i.import(&["--memory", &mem]);
    let cases = [
        (
            "working-memory/omit/feedback_short_names.md",
            "rule",
            "Short names",
            "Prefer short names for helpers",
        ),
        (
            "working-memory/omit/project_launch.md",
            "history",
            "Launch state",
            "Where the launch stands",
        ),
        (
            "working-memory/solo/reference_dashboard.md",
            "knowledge",
            "Dashboard",
            "Where the numbers live",
        ),
    ];
    for (rel, kind, title, first) in cases {
        let copy = i.copy(rel);
        assert_eq!(line(&copy, "kind"), Some(kind), "{rel}");
        assert_eq!(line(&copy, "title"), Some(title), "{rel}");
        assert_eq!(line(&copy, "visibility"), Some("private"), "{rel}");
        assert!(
            uuid::Uuid::parse_str(line(&copy, "id").unwrap()).is_ok(),
            "{rel}"
        );
        let body = copy.split("\n---\n").nth(1).unwrap();
        assert_eq!(body.lines().next(), Some(first), "{rel}");
    }
    let omit = i.h.vault.path().join("working-memory/omit");
    let mut names: Vec<_> = fs::read_dir(omit)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    assert_eq!(names, vec!["feedback_short_names.md", "project_launch.md"]);
}

pub(crate) fn link_dir(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        // cmd reads a `/` inside a path as a switch, so the paths go through as `\` only.
        let native = |p: &Path| p.components().collect::<PathBuf>();
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(native(link))
            .arg(native(target))
            .status()
            .unwrap()
            .success();
        assert!(ok, "mklink /J failed");
    }
}

#[test]
fn a_linked_memory_tree_is_read_once() {
    let i = Import::new();
    fs::create_dir_all(i.mem.path().join("twin")).unwrap();
    link_dir(
        &i.mem.path().join("omit/memory"),
        &i.mem.path().join("twin/memory"),
    );
    let mem = i.mem.path().to_str().unwrap().to_string();
    let out = i.import(&["--memory", &mem]);
    assert!(
        i.h.vault
            .path()
            .join("working-memory/omit/feedback_short_names.md")
            .exists()
    );
    assert!(!i.h.vault.path().join("working-memory/twin").exists());
    assert!(out.contains("memory/twin"), "{out}");
}

#[test]
fn the_report_lists_missing_sources_and_broken_links() {
    let i = Import::new();
    let out = i.import(&[]);
    let section = |name: &str| -> String {
        let start = out
            .find(name)
            .unwrap_or_else(|| panic!("no {name} in {out}"));
        out[start..].split("\n\n").next().unwrap().to_string()
    };
    assert!(
        section("missing a source").contains("2-literature/literature-note.md"),
        "{out}"
    );
    assert!(!section("missing a source").contains("kept-idea"), "{out}");
    assert!(section("broken links").contains("missing-note"), "{out}");
    assert!(
        !section("broken links").contains("literature-note"),
        "{out}"
    );
    assert!(
        section("inbox notes older than 14 days").contains("0-inbox/captured.md"),
        "{out}"
    );
    assert!(out.contains("private"), "{out}");
}

#[test]
fn a_long_report_list_says_how_many_more() {
    let i = Import::new();
    // The fixture's one broken link lives in kept-idea; this test counts exactly 25.
    fs::remove_file(i.src.path().join("3-permanent/kept-idea.md")).unwrap();
    for n in 0..25 {
        fs::write(
            i.src.path().join(format!("2-literature/broken{n:02}.md")),
            format!("---\nsource: s\n---\nsee [[gone-{n:02}]]\n"),
        )
        .unwrap();
    }
    let out = i.import(&[]);
    let gone = out.lines().filter(|l| l.contains("gone-")).count();
    assert_eq!(gone, 20, "{out}");
    assert!(out.contains("and 5 more"), "{out}");
}

#[test]
fn dry_run_writes_nothing() {
    let i = Import::new();
    let before = snapshot(i.h.vault.path());
    let mem = i.mem.path().to_str().unwrap().to_string();
    let out = i.import(&["--memory", &mem, "--dry-run"]);
    assert!(out.contains("new"), "{out}");
    assert_eq!(snapshot(i.h.vault.path()), before);
    assert!(!i.h.home.path().join("data/import-ledger.json").exists());
}

impl Import {
    /// Runs an import expected to fail; returns its stderr.
    fn import_fails(&self, extra: &[&str]) -> String {
        let mut args = vec![
            "--now",
            NOW,
            "import",
            "learning-loop",
            "--from",
            self.src(),
        ];
        args.extend_from_slice(extra);
        let out = self.h.brain().args(&args).output().unwrap();
        assert!(!out.status.success(), "brain {args:?} did not fail");
        String::from_utf8_lossy(&out.stderr).to_string()
    }

    fn put_source(&self, rel: &str, text: &str) {
        let p = self.src.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
}

#[test]
fn an_unknown_blocklist_tier_aborts_the_import() {
    let i = Import::new();
    let config = tempfile::tempdir().unwrap();
    fs::create_dir_all(config.path().join("federation")).unwrap();
    fs::write(
        config.path().join("federation/config.json"),
        r#"{"visibility": {"rules": [{"pattern": "**/*separation*", "tier": "Private"}]}}"#,
    )
    .unwrap();
    let before = snapshot(i.h.vault.path());
    let err = i.import_fails(&[
        "--share",
        "3-permanent",
        "--ll-config",
        config.path().to_str().unwrap(),
    ]);
    assert!(err.contains("Private"), "{err}");
    assert_eq!(snapshot(i.h.vault.path()), before);
}

#[test]
fn an_empty_or_escaping_share_folder_is_refused() {
    let i = Import::new();
    let before = snapshot(i.h.vault.path());
    let outside = i.h.home.path().to_str().unwrap().to_string();
    for bad in [
        "",
        ".",
        "./",
        "../3-permanent",
        "3-permanent/../..",
        outside.as_str(),
    ] {
        let err = i.import_fails(&["--share", bad, "--no-ll-config"]);
        assert!(err.contains("--share"), "{bad:?}: {err}");
    }
    assert_eq!(snapshot(i.h.vault.path()), before);
}

#[test]
fn a_share_folder_that_is_not_in_the_source_is_refused() {
    let i = Import::new();
    let before = snapshot(i.h.vault.path());
    for bad in ["3-Permanent", "4-projects", "3-permanent/kept-idea.md"] {
        let err = i.import_fails(&["--share", bad, "--no-ll-config"]);
        assert!(err.contains(bad), "{bad:?}: {err}");
    }
    assert_eq!(snapshot(i.h.vault.path()), before);
}

#[test]
fn a_share_folder_written_with_dot_or_slash_still_shares() {
    for spelling in ["./3-permanent", "3-permanent/"] {
        let i = Import::new();
        i.import(&["--share", spelling, "--no-ll-config"]);
        assert_eq!(
            line(&i.copy("3-permanent/kept-idea.md"), "visibility"),
            Some("shared"),
            "{spelling}"
        );
    }
}

#[test]
fn sharing_the_inbox_or_private_folder_still_imports_private() {
    let i = Import::new();
    i.put_source(
        "private/diary.md",
        "---\nid: 0190a0a0-0000-7000-8000-00000000d1a1\n---\nA private thought.\n",
    );
    i.import(&["--share", "0-inbox", "--share", "private", "--no-ll-config"]);
    assert_eq!(
        line(&i.copy("0-inbox/captured.md"), "visibility"),
        Some("private")
    );
    assert_eq!(
        line(&i.copy("private/diary.md"), "visibility"),
        Some("private")
    );
}

#[test]
fn a_misplaced_or_duplicate_visibility_is_kept_not_overridden() {
    let i = Import::new();
    let notes = [
        (
            "3-permanent/capital.md",
            "---\nVisibility: private\n---\nCapital V.\n",
        ),
        (
            "3-permanent/indented.md",
            "---\n  visibility: private\n---\nIndented.\n",
        ),
        (
            "3-permanent/twice.md",
            "---\nvisibility: private\nvisibility: shared\n---\nTwice.\n",
        ),
    ];
    for (rel, text) in notes {
        i.put_source(rel, text);
    }
    i.import(&["--share", "3-permanent", "--no-ll-config"]);
    for (rel, _) in notes {
        let copy = i.copy(rel);
        assert_eq!(
            copy.lines()
                .filter(|l| l.trim_start().to_lowercase().starts_with("visibility"))
                .count(),
            i.source(rel)
                .lines()
                .filter(|l| l.trim_start().to_lowercase().starts_with("visibility"))
                .count(),
            "{rel} gained a visibility line:\n{copy}"
        );
    }
}

#[test]
fn an_import_that_fails_part_way_resumes_on_the_next_run() {
    let i = Import::new();
    let blocker = i.h.vault.path().join("3-permanent/separation-agreement.md");
    fs::create_dir_all(&blocker).unwrap();
    i.import_fails(&[]);
    assert!(
        i.h.vault
            .path()
            .join("2-literature/literature-note.md")
            .exists()
    );
    fs::remove_dir(&blocker).unwrap();
    let out = i.import(&[]);
    assert!(out.contains("0 conflicts"), "{out}");
    assert!(blocker.is_file());
}

#[test]
fn a_linked_memory_project_is_skipped_and_named() {
    let i = Import::new();
    link_dir(&i.mem.path().join("solo"), &i.mem.path().join("echo"));
    let mem = i.mem.path().to_str().unwrap().to_string();
    let out = i.import(&["--memory", &mem]);
    assert!(
        i.h.vault
            .path()
            .join("working-memory/solo/reference_dashboard.md")
            .exists()
    );
    assert!(!i.h.vault.path().join("working-memory/echo").exists());
    assert!(out.contains("memory/echo"), "{out}");
}

#[cfg(unix)]
#[test]
fn a_linked_memory_file_is_skipped_and_named() {
    let i = Import::new();
    let dir = i.mem.path().join("solo/memory");
    std::os::unix::fs::symlink(
        dir.join("reference_dashboard.md"),
        dir.join("reference_twin.md"),
    )
    .unwrap();
    let mem = i.mem.path().to_str().unwrap().to_string();
    let out = i.import(&["--memory", &mem]);
    assert!(
        !i.h.vault
            .path()
            .join("working-memory/solo/reference_twin.md")
            .exists()
    );
    assert!(out.contains("reference_twin.md"), "{out}");
}

#[test]
fn a_vault_note_where_a_memory_note_goes_is_a_conflict() {
    let i = Import::new();
    i.put_source(
        "working-memory/solo/reference_dashboard.md",
        "A vault note in the way.\n",
    );
    let mem = i.mem.path().to_str().unwrap().to_string();
    let out = i.import(&["--memory", &mem]);
    assert!(out.contains("1 conflict"), "{out}");
    assert!(
        out.contains("working-memory/solo/reference_dashboard.md"),
        "{out}"
    );
    let copy = i.copy("working-memory/solo/reference_dashboard.md");
    let again = i.import(&["--memory", &mem]);
    assert_eq!(i.copy("working-memory/solo/reference_dashboard.md"), copy);
    assert!(again.contains("1 conflict"), "{again}");
}

#[test]
fn an_index_failure_after_the_copies_still_ledgers_them() {
    let i = Import::new();
    let db = i.h.home.path().join("data/index.db");
    let _ = fs::remove_file(&db);
    fs::create_dir_all(&db).unwrap();
    i.import_fails(&[]);
    fs::remove_dir(&db).unwrap();
    let out = i.import(&[]);
    assert!(out.contains("0 conflicts"), "{out}");
}

#[test]
fn a_quoted_memory_name_imports_as_written() {
    let i = Import::new();
    fs::write(
        i.mem.path().join("omit/memory/feedback_quoted.md"),
        "---\nname: \"say \\\"hi\\\": now\"\ndescription: d\ntype: feedback\n---\nx\n",
    )
    .unwrap();
    let mem = i.mem.path().to_str().unwrap().to_string();
    i.import(&["--memory", &mem]);
    let copy = i.copy("working-memory/omit/feedback_quoted.md");
    assert_eq!(
        line(&copy, "title"),
        Some("\"say \\\"hi\\\": now\""),
        "{copy}"
    );
}

/// An import of a few notes into a large vault once listed the whole vault's gaps, which reads
/// like data loss.
#[test]
fn an_import_reports_only_on_its_own_notes() {
    let h = Home::new();
    h.init();
    h.put("elsewhere.md", "no source here, and [[nowhere-at-all]]\n");
    h.put("target.md", "---\nsource: me\n---\nsomething\n");
    let from = tempfile::tempdir().unwrap();
    fs::write(
        from.path().join("brought.md"),
        "---\nsource: me\n---\nsee [[target]] and [[no-such-note]]\n",
    )
    .unwrap();
    let out = h.run(&[
        "import",
        "learning-loop",
        "--from",
        from.path().to_str().unwrap(),
    ]);
    assert!(!out.contains("elsewhere.md"), "{out}");
    assert!(!out.contains("nowhere-at-all"), "{out}");
    assert!(!out.contains("[[target]]"), "{out}");
    assert!(out.contains("brought.md -> [[no-such-note]]"), "{out}");
    assert!(
        out.contains("visibility: 0 shared, 0 listed, 1 private"),
        "{out}"
    );
}

#[test]
fn a_copy_keeps_its_sources_date() {
    let i = Import::new();
    let note = "3-permanent/kept-idea.md";
    let memory = i.mem.path().join("omit/memory/project_launch.md");
    super::set_changed(&i.src.path().join(note), "2026-03-05T12:00:00Z");
    super::set_changed(&memory, "2026-04-06T12:00:00Z");
    let mem = i.mem.path().to_str().unwrap().to_string();
    i.import(&["--memory", &mem]);
    let vault = i.h.vault.path();
    assert_eq!(
        super::changed_at(&vault.join(note)),
        super::changed_at(&i.src.path().join(note))
    );
    assert_eq!(
        super::changed_at(&vault.join("working-memory/omit/project_launch.md")),
        super::changed_at(&memory)
    );
}
