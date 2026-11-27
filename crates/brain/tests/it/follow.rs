//! `brain memory follow`: the copies of Claude Code's auto-memory stay in step with their
//! sources, and nothing is taken away on doubt.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use brain_platform::fs::Lock;

use super::Home;

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

/// The archive takes the note's text out of the traces; the record of the archiving stays.
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
        .flat_map(|r| store.spans(&r.id).unwrap())
        .collect();
    assert!(spans.iter().any(|s| s.name == "archive"), "{spans:?}");
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
