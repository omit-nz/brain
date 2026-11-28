//! `brain memory follow`: the copies of Claude Code's auto-memory stay in step with their
//! sources, and nothing is taken away on doubt.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use brain_platform::fs::Lock;

use super::Home;

mod restore;

/// Claude Code's projects folder, with one project's memory.
struct Projects {
    dir: tempfile::TempDir,
}

impl Projects {
    fn new() -> Projects {
        Projects {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path().join("-proj/memory").join(format!("{name}.md"))
    }

    fn write(&self, name: &str, text: &str) {
        let p = self.file(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(
            p,
            format!("---\nname: {name}\ndescription: {text}\ntype: project\n---\n"),
        )
        .unwrap();
    }

    fn remove(&self, name: &str) {
        fs::remove_file(self.file(name)).unwrap();
    }
}

fn copy_path(vault: &Path, name: &str) -> PathBuf {
    vault
        .join("working-memory/-proj")
        .join(format!("{name}.md"))
}

fn copy(h: &Home, name: &str) -> Option<String> {
    fs::read_to_string(copy_path(h.vault.path(), name)).ok()
}

fn archived(h: &Home, name: &str) -> bool {
    h.vault
        .path()
        .join("_archive/working-memory/-proj")
        .join(format!("{name}.md"))
        .exists()
}

fn follow(h: &Home, p: &Projects) -> String {
    h.run(&["memory", "follow", p.path().to_str().unwrap()])
}

/// One pass. A source is only taken for gone when two passes in a row find it so.
fn pass(h: &Home) -> String {
    h.run(&["memory", "run"])
}

fn following(names: &[&str]) -> (Home, Projects) {
    let h = Home::new();
    h.init();
    let p = Projects::new();
    for name in names {
        p.write(name, &format!("{name} first"));
    }
    follow(&h, &p);
    (h, p)
}

#[test]
fn a_new_memory_is_copied() {
    let (h, p) = following(&["one"]);
    assert!(copy(&h, "one").unwrap().contains("one first"));
    p.write("two", "two first");
    let said = pass(&h);
    assert!(said.contains("1 copied"), "{said}");
    assert!(copy(&h, "two").unwrap().contains("two first"));
}

#[test]
fn a_file_already_where_a_copy_would_go_is_kept() {
    let (h, p) = following(&["one"]);
    h.put("working-memory/-proj/two.md", "mine\n");
    p.write("two", "two first");
    let said = pass(&h);
    assert!(copy(&h, "two").unwrap().ends_with("mine\n"));
    assert!(said.contains("working-memory/-proj/two.md"), "{said}");
}

#[test]
fn a_changed_memory_replaces_its_copy() {
    let (h, p) = following(&["one"]);
    let id = |text: &str| {
        text.lines()
            .find(|l| l.starts_with("id:"))
            .unwrap()
            .to_string()
    };
    let before = id(&copy(&h, "one").unwrap());
    p.write("one", "one second");
    pass(&h);
    let after = copy(&h, "one").unwrap();
    assert!(after.contains("one second"), "{after}");
    assert_eq!(id(&after), before);
}

#[test]
fn a_copy_the_person_removed_stays_removed() {
    let (h, _p) = following(&["one"]);
    fs::remove_file(copy_path(h.vault.path(), "one")).unwrap();
    pass(&h);
    assert_eq!(copy(&h, "one"), None);
}

#[test]
fn an_edited_copy_survives_its_source_changing() {
    let (h, p) = following(&["one"]);
    h.put("working-memory/-proj/one.md", "my own words\n");
    p.write("one", "one second");
    let said = pass(&h);
    assert!(copy(&h, "one").unwrap().ends_with("my own words\n"));
    assert!(said.contains("working-memory/-proj/one.md"), "{said}");
}

#[test]
fn a_gone_source_takes_its_copy_out_of_search() {
    let (h, p) = following(&["one", "two"]);
    p.remove("one");
    pass(&h);
    assert!(copy(&h, "one").is_some(), "one pass is not enough");
    let said = pass(&h);
    assert!(said.contains("1 archived"), "{said}");
    assert_eq!(copy(&h, "one"), None);
    assert!(archived(&h, "one"));
    assert!(copy(&h, "two").is_some());
    let found = h.json(&["search", "--json", "first"]);
    let paths: Vec<&str> = found["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["working-memory/-proj/two.md"]);
}

#[test]
fn a_source_that_came_back_between_passes_is_not_archived() {
    let (h, p) = following(&["one", "two"]);
    p.remove("one");
    pass(&h);
    p.write("one", "one first");
    pass(&h);
    p.remove("one");
    pass(&h);
    assert!(copy(&h, "one").is_some());
}

#[test]
fn an_edited_copy_survives_its_source_going() {
    let (h, p) = following(&["one", "two"]);
    h.put("working-memory/-proj/one.md", "my own words\n");
    p.remove("one");
    pass(&h);
    let said = pass(&h);
    assert!(said.contains("working-memory/-proj/one.md"), "{said}");
    assert!(copy(&h, "one").unwrap().ends_with("my own words\n"));
    // Forgotten after that: it is the person's note now.
    let again = pass(&h);
    assert!(!again.contains("one.md"), "{again}");
}

/// No marker holds a pass, and no pass dates a copy: not one made, changed or brought back.
#[test]
fn follow_runs_over_a_stray_clearing_marker_and_dates_nothing() {
    let (h, p) = following(&["one", "two"]);
    fs::write(h.home.path().join("data/clearing.json"), "[\"2026-11-25\"]").unwrap();
    p.remove("one");
    pass(&h);
    pass(&h);
    assert!(archived(&h, "one"));
    p.write("one", "one first");
    p.write("two", "two second");
    p.write("three", "three first");
    let said = follow(&h, &p);
    for did in ["1 copied", "1 updated", "1 restored"] {
        assert!(said.contains(did), "{said}");
    }
    for name in ["one", "two", "three"] {
        let text = copy(&h, name).unwrap();
        assert!(!text.contains("review:"), "{text}");
    }
}

#[test]
fn a_missing_or_empty_memory_folder_removes_nothing() {
    let (h, p) = following(&["one"]);
    p.remove("one");
    for _ in 0..3 {
        let out = h.brain().args(["memory", "run"]).output().unwrap();
        assert!(!out.status.success(), "{out:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("nothing changed"));
    }
    assert!(copy(&h, "one").is_some());
    let gone = p.path().to_path_buf();
    drop(p);
    let out = h.brain().args(["memory", "run"]).output().unwrap();
    assert!(!out.status.success(), "{}", gone.display());
    assert!(copy(&h, "one").is_some());
}

#[test]
fn an_unreadable_source_removes_nothing() {
    let (h, p) = following(&["one", "two"]);
    fs::write(p.file("one"), [0xff, 0xfe, 0x00]).unwrap();
    pass(&h);
    pass(&h);
    assert!(copy(&h, "one").is_some());
}

#[cfg(unix)]
#[test]
fn an_unsearchable_project_folder_removes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let (h, p) = following(&["one"]);
    p.write("two", "two first");
    let other = p.path().join("-other/memory");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("kept.md"), "---\nname: kept\n---\n").unwrap();
    pass(&h);
    let project = p.path().join("-proj");
    let mode = |m| fs::set_permissions(&project, fs::Permissions::from_mode(m)).unwrap();
    mode(0o000);
    pass(&h);
    pass(&h);
    mode(0o755);
    assert!(copy(&h, "one").is_some() && copy(&h, "two").is_some());
}

#[test]
fn following_after_a_one_time_import_copies_nothing_twice() {
    let h = Home::new();
    h.init();
    let p = Projects::new();
    p.write("one", "one first");
    import(&h, p.path());
    let said = follow(&h, &p);
    assert!(said.contains("0 copied, 0 updated, 0 archived"), "{said}");
    // The one-time import's copy is followed from here on.
    p.write("one", "one second");
    pass(&h);
    assert!(copy(&h, "one").unwrap().contains("one second"));
}

/// `brain import` with a one-note vault beside the memory folder.
fn import(h: &Home, memory: &Path) {
    let from = tempfile::tempdir().unwrap();
    fs::write(from.path().join("a.md"), "a\n").unwrap();
    h.run(&[
        "import",
        "learning-loop",
        "--from",
        from.path().to_str().unwrap(),
        "--memory",
        memory.to_str().unwrap(),
    ]);
}

#[test]
fn a_follow_never_archives_what_another_folder_gave() {
    let h = Home::new();
    h.init();
    let backup = Projects::new();
    backup.write("old", "old first");
    import(&h, backup.path());
    let live = Projects::new();
    live.write("new", "new first");
    follow(&h, &live);
    pass(&h);
    pass(&h);
    assert!(copy(&h, "old").is_some());
}

#[test]
fn an_import_from_elsewhere_survives_the_next_pass() {
    let (h, _p) = following(&["one"]);
    let other = Projects::new();
    other.write("extra", "extra first");
    import(&h, other.path());
    pass(&h);
    pass(&h);
    assert!(copy(&h, "extra").is_some() && copy(&h, "one").is_some());
}

#[test]
fn following_into_a_new_vault_copies_every_memory() {
    let (h, p) = following(&["one"]);
    let next = tempfile::tempdir().unwrap();
    h.run(&[
        "init",
        "--me",
        "robin",
        "--vault",
        next.path().to_str().unwrap(),
    ]);
    // A second init keeps what is followed.
    pass(&h);
    assert!(copy_path(next.path(), "one").exists());
    assert!(p.file("one").exists());
}

#[test]
fn a_missing_vault_is_never_recreated() {
    let (h, p) = following(&["one"]);
    let vault = h.vault.path().to_path_buf();
    fs::remove_dir_all(&vault).unwrap();
    p.write("two", "two first");
    let out = h
        .brain()
        .current_dir(h.home.path())
        .args(["memory", "run"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(!vault.exists());
}

#[test]
fn unfollow_leaves_the_copies_and_follows_no_more() {
    let (h, p) = following(&["one"]);
    h.run(&["memory", "unfollow"]);
    p.write("two", "two first");
    let out = h.brain().args(["memory", "run"]).output().unwrap();
    assert!(!out.status.success());
    assert!(copy(&h, "one").is_some());
    assert_eq!(copy(&h, "two"), None);
}

/// The ledger is read, changed and saved under one lock.
#[test]
fn an_import_and_a_follow_never_write_the_ledger_at_once() {
    let (h, p) = following(&["one"]);
    let held = Lock::acquire(&h.home.path().join("data/import.lock")).unwrap();
    let follow: &[&str] = &["memory", "run"];
    let vault = tempfile::tempdir().unwrap();
    fs::write(vault.path().join("a.md"), "a\n").unwrap();
    let import: &[&str] = &[
        "import",
        "learning-loop",
        "--from",
        vault.path().to_str().unwrap(),
    ];
    let mut waiting: Vec<_> = [follow, import]
        .into_iter()
        .map(|args| {
            std::process::Command::new(assert_cmd::cargo::cargo_bin("brain"))
                .args(args)
                .env("BRAIN_HOME", h.home.path())
                .env_remove("ORT_DYLIB_PATH")
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    std::thread::sleep(Duration::from_millis(700));
    for child in &mut waiting {
        assert!(child.try_wait().unwrap().is_none(), "ran under a held lock");
    }
    drop(held);
    for child in &mut waiting {
        assert!(child.wait().unwrap().success());
    }
    assert!(p.file("one").exists());
}

#[test]
fn a_sync_pass_is_traced() {
    let (h, p) = following(&["one"]);
    let store = omit_agent_trace::Store::open(&h.home.path().join("data/traces.db")).unwrap();
    let runs = || {
        store
            .runs()
            .unwrap()
            .into_iter()
            .filter(|r| r.name == "memory follow")
            .collect::<Vec<_>>()
    };
    assert_eq!(runs().len(), 1);
    pass(&h);
    assert_eq!(runs().len(), 1, "a pass that changes nothing is not traced");
    p.write("one", "one second");
    pass(&h);
    let spans = store.spans(&runs()[0].id).unwrap();
    let names: Vec<_> = spans.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["update", "source"]);
    let file = format!("file:{}", p.file("one").canonicalize().unwrap().display());
    assert!(spans[1].sources.contains(&file), "{:?}", spans[1].sources);
    assert!(spans[1].sources.iter().any(|s| s.starts_with("note:")));
}

/// A copy made before the ledger kept the source folder could be from anywhere.
#[test]
fn a_copy_of_unknown_origin_is_never_archived() {
    let (h, p) = following(&["one", "two"]);
    let path = h.home.path().join("data/import-ledger.json");
    let mut ledger: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    for entry in ledger.as_object_mut().unwrap().values_mut() {
        let entry = entry.as_object_mut().unwrap();
        entry.remove("from");
        entry.remove("vault");
    }
    fs::write(&path, ledger.to_string()).unwrap();
    p.remove("one");
    pass(&h);
    pass(&h);
    pass(&h);
    assert!(copy(&h, "one").is_some());
}

/// The archiving pass records what it did without the words it took away.
#[test]
fn a_pass_that_archives_keeps_its_trace() {
    let (h, p) = following(&["one", "two"]);
    p.remove("one");
    pass(&h);
    pass(&h);
    let store = omit_agent_trace::Store::open(&h.home.path().join("data/traces.db")).unwrap();
    let runs: Vec<_> = store
        .runs()
        .unwrap()
        .into_iter()
        .filter(|r| r.name == "memory follow")
        .collect();
    let spans: Vec<_> = runs
        .iter()
        .map(|r| store.spans(&r.id).unwrap())
        .find(|spans| spans.iter().any(|s| s.name == "archive"))
        .expect("a run that archived");
    assert!(!format!("{spans:?}").contains("one first"), "{spans:?}");
}

#[cfg(unix)]
mod renamed {
    use std::os::unix::fs::symlink;

    use super::*;

    /// Moves `-proj/memory` to `-new/memory` and leaves a link behind.
    fn rename_project(p: &Projects) {
        let new = p.path().join("-new");
        fs::create_dir_all(&new).unwrap();
        fs::rename(p.path().join("-proj/memory"), new.join("memory")).unwrap();
        symlink(new.join("memory"), p.path().join("-proj/memory")).unwrap();
    }

    fn new_source(p: &Projects, text: &str) {
        fs::write(
            p.path().join("-new/memory/one.md"),
            format!("---\nname: one\ndescription: {text}\ntype: project\n---\n"),
        )
        .unwrap();
    }

    fn new_copy(h: &Home) -> Option<String> {
        fs::read_to_string(h.vault.path().join("working-memory/-new/one.md")).ok()
    }

    fn hits(h: &Home, query: &str) -> usize {
        h.json(&["search", "--json", query])["hits"]
            .as_array()
            .unwrap()
            .len()
    }

    #[test]
    fn a_memory_folder_left_behind_a_link_moves_its_copies() {
        let (h, p) = following(&["one"]);
        let before = copy(&h, "one").unwrap();
        rename_project(&p);
        pass(&h);
        assert_eq!(new_copy(&h), Some(before));
        assert_eq!(copy(&h, "one"), None);
        assert!(!archived(&h, "one"), "moved, not archived");
        assert_eq!(hits(&h, "first"), 1);
        new_source(&p, "one second");
        pass(&h);
        pass(&h);
        assert!(new_copy(&h).unwrap().contains("one second"));
        assert_eq!(copy(&h, "one"), None);
    }

    /// As after a one-time import, whose ledger names no source folder.
    #[test]
    fn a_renamed_project_moves_copies_of_unknown_origin() {
        let (h, p) = following(&["one"]);
        let path = h.home.path().join("data/import-ledger.json");
        let mut ledger: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for entry in ledger.as_object_mut().unwrap().values_mut() {
            let entry = entry.as_object_mut().unwrap();
            entry.remove("from");
            entry.remove("vault");
        }
        fs::write(&path, ledger.to_string()).unwrap();
        rename_project(&p);
        pass(&h);
        assert!(new_copy(&h).unwrap().contains("one first"));
        assert_eq!(copy(&h, "one"), None);
    }

    /// The move can come before brain follows anything: a one-time import by a release with no
    /// follow, the project renamed with a link left behind, and only then the first pass.
    #[test]
    fn a_project_moved_between_the_import_and_the_first_follow_is_the_same_notes() {
        let h = Home::new();
        h.init();
        let p = Projects::new();
        for name in ["one", "two", "three"] {
            p.write(name, &format!("{name} first"));
        }
        import(&h, p.path());
        // The ledger as the release without follow wrote it: no source folder, no vault.
        let path = h.home.path().join("data/import-ledger.json");
        let mut ledger: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for entry in ledger.as_object_mut().unwrap().values_mut() {
            let entry = entry.as_object_mut().unwrap();
            entry.remove("from");
            entry.remove("vault");
        }
        fs::write(&path, ledger.to_string()).unwrap();
        let before = ["one", "two", "three"].map(|n| copy(&h, n).unwrap());
        fs::create_dir_all(p.path().join("-new")).unwrap();
        fs::rename(p.path().join("-proj/memory"), p.path().join("-new/memory")).unwrap();
        symlink("../-new/memory", p.path().join("-proj/memory")).unwrap();
        // Changed since the import, and written since the import.
        new_source(&p, "one second");
        fs::write(p.path().join("-new/memory/four.md"), "four first\n").unwrap();
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("projects");
        symlink(p.path(), &link).unwrap();
        let said = h.run(&["memory", "follow", link.to_str().unwrap()]);
        assert!(
            said.contains("1 copied, 1 updated, 0 archived, 3 moved"),
            "{said}"
        );
        assert!(said.contains("0 skipped"), "the link is not news: {said}");
        let new = |n: &str| {
            fs::read_to_string(h.vault.path().join(format!("working-memory/-new/{n}.md"))).unwrap()
        };
        assert_eq!(
            [new("two"), new("three")],
            [before[1].clone(), before[2].clone()]
        );
        let id = |text: &str| text.lines().nth(1).unwrap().to_string();
        assert_eq!(id(&new("one")), id(&before[0]));
        assert!(new("one").contains("one second"));
        assert!(!h.vault.path().join("working-memory/-proj/one.md").exists());
        for query in ["second", "two", "three"] {
            assert_eq!(hits(&h, query), 1, "{query}");
        }
        for _ in 0..2 {
            let said = pass(&h);
            assert!(
                said.contains("0 copied, 0 updated, 0 archived, 0 moved"),
                "{said}"
            );
        }
    }

    /// A pass killed after it moved a copy and before it saved the ledger.
    #[test]
    fn a_rename_cut_short_is_finished_by_the_next_pass() {
        let (h, p) = following(&["one"]);
        rename_project(&p);
        let moved = h.vault.path().join("working-memory/-new");
        fs::create_dir_all(&moved).unwrap();
        fs::rename(copy_path(h.vault.path(), "one"), moved.join("one.md")).unwrap();
        pass(&h);
        new_source(&p, "one second");
        let said = pass(&h);
        assert!(said.contains("1 updated"), "{said}");
        assert!(new_copy(&h).unwrap().contains("one second"));
    }

    #[test]
    fn an_edited_or_removed_copy_stays_so_after_a_rename() {
        let (h, p) = following(&["one", "two"]);
        h.put("working-memory/-proj/one.md", "my own words\n");
        fs::remove_file(copy_path(h.vault.path(), "two")).unwrap();
        rename_project(&p);
        pass(&h);
        assert!(new_copy(&h).unwrap().ends_with("my own words\n"));
        assert!(!h.vault.path().join("working-memory/-new/two.md").exists());
    }

    /// Both names were copied before the old one became a link: one copy is left in search.
    #[test]
    fn a_second_copy_of_a_renamed_project_leaves_search() {
        let (h, p) = following(&["one"]);
        fs::create_dir_all(p.path().join("-new/memory")).unwrap();
        new_source(&p, "one first");
        pass(&h);
        assert!(new_copy(&h).is_some());
        fs::remove_dir_all(p.path().join("-proj/memory")).unwrap();
        symlink(p.path().join("-new/memory"), p.path().join("-proj/memory")).unwrap();
        pass(&h);
        assert_eq!(copy(&h, "one"), None);
        assert!(archived(&h, "one"));
        assert!(new_copy(&h).is_some());
    }

    #[test]
    fn a_link_that_leads_out_of_the_followed_folder_changes_nothing() {
        let (h, p) = following(&["one"]);
        let elsewhere = tempfile::tempdir().unwrap();
        let memory = elsewhere.path().join("memory");
        fs::rename(p.path().join("-proj/memory"), &memory).unwrap();
        symlink(&memory, p.path().join("-proj/memory")).unwrap();
        // Something else to read, so the folder is not taken for an empty one.
        fs::create_dir_all(p.path().join("-other/memory")).unwrap();
        fs::write(p.path().join("-other/memory/x.md"), "x\n").unwrap();
        pass(&h);
        pass(&h);
        assert!(copy(&h, "one").unwrap().contains("one first"));
    }

    #[test]
    fn a_link_to_the_projects_folder_is_followed_as_what_it_points_at() {
        let h = Home::new();
        h.init();
        let p = Projects::new();
        p.write("one", "one first");
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("projects");
        symlink(p.path(), &link).unwrap();
        h.run(&["memory", "follow", link.to_str().unwrap()]);
        assert!(copy(&h, "one").is_some());
        // The link going does not: what it pointed at is what is followed.
        fs::remove_file(&link).unwrap();
        p.write("one", "one second");
        pass(&h);
        assert!(copy(&h, "one").unwrap().contains("one second"));
    }
}

#[test]
fn a_followed_copy_keeps_its_sources_date() {
    let h = Home::new();
    h.init();
    let p = Projects::new();
    p.write("one", "one first");
    crate::set_changed(&p.file("one"), "2026-03-05T12:00:00Z");
    follow(&h, &p);
    let at = || crate::changed_at(&copy_path(h.vault.path(), "one"));
    assert_eq!(at(), crate::changed_at(&p.file("one")));
    p.write("one", "one second");
    crate::set_changed(&p.file("one"), "2026-04-06T12:00:00Z");
    pass(&h);
    assert_eq!(at(), crate::changed_at(&p.file("one")));
}

/// A copy made before copies took their source's date says when brain copied it, which is later
/// than the words are.
#[test]
fn a_pass_dates_an_older_copy_back_and_never_forward() {
    let (h, p) = following(&["one"]);
    let (source, copy) = (p.file("one"), copy_path(h.vault.path(), "one"));
    crate::set_changed(&source, "2026-03-05T12:00:00Z");
    pass(&h);
    assert_eq!(crate::changed_at(&copy), crate::changed_at(&source));
    // Touched, as a move or a checkout does, with the words as they were.
    crate::set_changed(&source, "2026-06-07T12:00:00Z");
    pass(&h);
    assert!(crate::changed_at(&copy) < crate::changed_at(&source));
}

#[test]
fn a_copy_whose_source_is_deleted_keeps_its_date() {
    let h = Home::new();
    h.init();
    let p = Projects::new();
    p.write("one", "one first");
    p.write("two", "two first");
    crate::set_changed(&p.file("one"), "2026-03-05T12:00:00Z");
    import(&h, p.path());
    let copied = copy_path(h.vault.path(), "one");
    let before = crate::changed_at(&copied);
    p.remove("one");
    let said = follow(&h, &p);
    assert!(said.contains("0 skipped"), "{said}");
    assert_eq!(crate::changed_at(&copied), before);
}

/// Two projects each with a memory of one name, the old one's folder then linked to the other's:
/// neither's words are lost.
#[cfg(unix)]
#[test]
fn two_memories_of_one_name_keep_their_words_when_the_projects_are_linked() {
    let (h, p) = following(&["one"]);
    let other = p.path().join("-new/memory");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("one.md"), "another memory entirely\n").unwrap();
    pass(&h);
    fs::remove_dir_all(p.path().join("-proj/memory")).unwrap();
    std::os::unix::fs::symlink(&other, p.path().join("-proj/memory")).unwrap();
    pass(&h);
    pass(&h);
    pass(&h);
    let records = fs::read_to_string(h.home.path().join("data/follow-archive.json")).unwrap();
    assert!(records.contains("memory/-proj/one.md"), "{records}");
    // Wherever brain keeps them: the one in the vault or its archive, the live one in search.
    let mut dirs = vec![h.vault.path().to_path_buf()];
    let mut files = Vec::new();
    while let Some(dir) = dirs.pop() {
        for e in fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
            if e.is_dir() {
                dirs.push(e);
            } else {
                files.push(fs::read_to_string(e).unwrap_or_default());
            }
        }
    }
    assert!(files.iter().any(|text| text.contains("one first")));
    let found = h.json(&["search", "--json", "entirely"]);
    assert_eq!(found["hits"].as_array().unwrap().len(), 1, "{found}");
}

#[cfg(unix)]
mod links {
    use std::os::unix::fs::symlink;

    use super::*;

    /// The memories of `-proj`, copied under `-new` too, with other words.
    fn also_under_new(p: &Projects, names: &[String]) {
        let new = p.path().join("-new/memory");
        fs::create_dir_all(&new).unwrap();
        for name in names {
            fs::write(
                new.join(format!("{name}.md")),
                format!("other words {name}\n"),
            )
            .unwrap();
        }
    }

    fn link_old_to_new(p: &Projects) {
        fs::remove_dir_all(p.path().join("-proj/memory")).unwrap();
        symlink(p.path().join("-new/memory"), p.path().join("-proj/memory")).unwrap();
    }

    /// Every markdown file's text anywhere in the vault, the archive included.
    fn vault_texts(h: &Home) -> Vec<String> {
        let mut dirs = vec![h.vault.path().to_path_buf()];
        let mut texts = Vec::new();
        while let Some(dir) = dirs.pop() {
            for e in fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()) {
                if e.is_dir() {
                    dirs.push(e);
                } else {
                    texts.push(fs::read_to_string(e).unwrap_or_default());
                }
            }
        }
        texts
    }

    /// A person tidies away the second copy, then links: the old copy is the only one left.
    #[test]
    fn a_copy_that_is_the_only_one_left_is_moved_not_archived() {
        let (h, p) = following(&["one"]);
        fs::create_dir_all(p.path().join("-new/memory")).unwrap();
        fs::copy(p.file("one"), p.path().join("-new/memory/one.md")).unwrap();
        pass(&h);
        let second = h.vault.path().join("working-memory/-new/one.md");
        fs::remove_file(&second).unwrap();
        link_old_to_new(&p);
        pass(&h);
        pass(&h);
        assert!(second.exists());
        assert!(!archived(&h, "one"));
    }

    /// Two memories of one name with different words, one copy tidied away: neither the move nor
    /// anything after it writes over the other's words.
    #[test]
    fn a_move_never_writes_over_a_memorys_words() {
        let (h, p) = following(&["one"]);
        also_under_new(&p, &["one".to_string()]);
        pass(&h);
        fs::remove_file(h.vault.path().join("working-memory/-new/one.md")).unwrap();
        link_old_to_new(&p);
        for _ in 0..3 {
            pass(&h);
        }
        assert!(vault_texts(&h).iter().any(|t| t.contains("one first")));
        let records = fs::read_to_string(h.home.path().join("data/follow-archive.json")).unwrap();
        assert!(records.contains("memory/-proj/one.md"), "{records}");
        let ledger = fs::read_to_string(h.home.path().join("data/import-ledger.json")).unwrap();
        assert!(!ledger.contains("memory/-proj/one.md"), "{ledger}");
        assert!(
            !h.vault.path().join("working-memory/-new/one.md").exists(),
            "a removed copy stays removed"
        );
    }

    /// A memory folder linked to a disk of its own, and the disk goes away for a while.
    #[test]
    fn a_link_whose_target_goes_away_archives_nothing() {
        let (h, p) = following(&["one"]);
        fs::create_dir_all(p.path().join("-other/memory")).unwrap();
        fs::write(p.path().join("-other/memory/x.md"), "x\n").unwrap();
        let disk = tempfile::tempdir().unwrap();
        let away = disk.path().join("memory");
        fs::rename(p.path().join("-proj/memory"), &away).unwrap();
        symlink(&away, p.path().join("-proj/memory")).unwrap();
        pass(&h);
        fs::rename(&away, disk.path().join("unplugged")).unwrap();
        for _ in 0..3 {
            pass(&h);
        }
        assert!(copy(&h, "one").is_some());
        assert!(!archived(&h, "one"));
    }
}
