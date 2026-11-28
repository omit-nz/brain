mod brief;
mod capture;
// The doors run on macOS; their stubs are shell scripts.
#[cfg(unix)]
mod connect;
mod daemon;
mod follow;
mod gaps;
mod hook;
mod import;
mod mcp;
mod room;
mod service;
mod update;
mod usage;
mod vault;

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// A file's hash as brain's records keep it.
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Home {
    home: tempfile::TempDir,
    vault: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        Home {
            home: tempfile::tempdir().unwrap(),
            vault: tempfile::tempdir().unwrap(),
        }
    }

    fn brain(&self) -> Command {
        let mut c = Command::cargo_bin("brain").unwrap();
        c.env("BRAIN_HOME", self.home.path())
            // A loaded CI runner must not turn a hook test into a timing test.
            .env("BRAIN_TEST_HOOK_BUDGET_MS", "30000")
            .env_remove("ORT_DYLIB_PATH")
            .env_remove("LL_MODELS_DIR")
            .current_dir(self.vault.path());
        c
    }

    fn run(&self, args: &[&str]) -> String {
        let out = self.brain().args(args).output().unwrap();
        assert!(
            out.status.success(),
            "brain {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn init(&self) {
        self.run(&[
            "init",
            "--me",
            "robin",
            "--vault",
            self.vault.path().to_str().unwrap(),
        ]);
        // No test reaches the real endpoint: only the usage tests turn sending on, at their own
        // listener.
        self.usage("send = false");
    }

    /// Replaces the config's `[usage]` table with `table`.
    fn usage(&self, table: &str) {
        let path = self.home.path().join("config/config.toml");
        let config = fs::read_to_string(&path).unwrap();
        let rest = config.split("[usage]").next().unwrap().trim_end();
        fs::write(&path, format!("{rest}\n\n[usage]\n{table}\n")).unwrap();
    }

    fn put(&self, rel: &str, text: &str) {
        let p = self.vault.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.run(args)).unwrap()
    }

    fn inbox(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.vault.path().join("0-inbox"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }
}

#[test]
fn init_then_status_reports_an_empty_vault() {
    let h = Home::new();
    h.init();
    for folder in ["0-inbox", "private", "working-memory"] {
        assert!(h.vault.path().join(folder).is_dir(), "{folder}");
    }
    let s = h.json(&["status", "--json"]);
    assert_eq!(s["notes"], 0);
    assert_eq!(s["pending"], 0);
    assert_eq!(
        Path::new(s["vault"].as_str().unwrap()),
        h.vault.path().canonicalize().unwrap()
    );
    assert_eq!(s["version"], env!("CARGO_PKG_VERSION"));
}

/// brain writes ids into every note under its vault, so a home folder is refused as one whether
/// named or the folder init runs in, and a code repository when init falls back to the folder it
/// runs in.
#[test]
fn init_refuses_a_home_folder_or_a_code_repository() {
    let h = Home::new();
    let fake_home = tempfile::tempdir().unwrap();
    fs::create_dir_all(fake_home.path().join("dev")).unwrap();
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join(".git")).unwrap();
    for (cwd, vault) in [
        (fake_home.path(), None),
        (h.vault.path(), Some(fake_home.path())),
        (repo.path(), None),
    ] {
        let mut args = vec!["init", "--me", "robin"];
        if let Some(v) = vault {
            args.extend(["--vault", v.to_str().unwrap()]);
        }
        let out = h
            .brain()
            .env("HOME", fake_home.path())
            .env("USERPROFILE", fake_home.path())
            .current_dir(cwd)
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{cwd:?} {vault:?}");
        assert!(String::from_utf8_lossy(&out.stderr).contains("--vault"));
    }
    assert_eq!(fs::read_dir(fake_home.path()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(repo.path()).unwrap().count(), 1);
    assert!(!h.home.path().join("config").exists());
}

/// A notes folder synced with git (obsidian-git keeps `.git` in the vault root) is a vault when
/// the person names it.
#[test]
fn a_named_vault_holding_git_is_accepted() {
    let h = Home::new();
    fs::create_dir_all(h.vault.path().join(".git")).unwrap();
    h.put("a.md", "a note\n");
    let elsewhere = tempfile::tempdir().unwrap();
    let out = h
        .brain()
        .current_dir(elsewhere.path())
        .args([
            "init",
            "--me",
            "robin",
            "--vault",
            h.vault.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(h.vault.path().join("0-inbox").is_dir());
    assert_eq!(
        fs::read_to_string(h.vault.path().join("a.md")).unwrap(),
        "a note\n"
    );
}

#[test]
fn init_never_touches_existing_notes() {
    let h = Home::new();
    let notes = [
        ("a.md", "no frontmatter\n"),
        ("private/b.md", "---\ntitle: b\n---\nx\n"),
    ];
    for (rel, text) in notes {
        h.put(rel, text);
    }
    h.init();
    for (rel, text) in notes {
        assert_eq!(fs::read_to_string(h.vault.path().join(rel)).unwrap(), text);
    }
}

#[test]
fn add_lands_in_the_inbox_with_an_id() {
    let h = Home::new();
    h.init();
    h.run(&[
        "add",
        "The kettle descaler lives under the sink",
        "--title",
        "Kettle",
    ]);
    let names = h.inbox();
    assert_eq!(names, vec!["kettle.md"]);
    let text = fs::read_to_string(h.vault.path().join("0-inbox/kettle.md")).unwrap();
    let id = text.lines().find_map(|l| l.strip_prefix("id: ")).unwrap();
    assert_eq!(uuid::Uuid::parse_str(id).unwrap().get_version_num(), 7);
    assert!(text.contains("title: Kettle\n"));
    assert!(text.ends_with("The kettle descaler lives under the sink\n"));
    h.run(&["add", "a second kettle note", "--title", "Kettle"]);
    assert_eq!(h.inbox(), vec!["kettle-1.md", "kettle.md"]);
}

#[test]
fn add_sets_owner_and_review() {
    let h = Home::new();
    h.init();
    h.run(&[
        "--now",
        "2026-11-25T03:00:00Z",
        "add",
        "remember the bins go out Tuesday",
        "--kind",
        "rule",
    ]);
    let name = &h.inbox()[0];
    let text = fs::read_to_string(h.vault.path().join("0-inbox").join(name)).unwrap();
    assert!(text.contains("owner: robin\n"), "{text}");
    assert!(text.contains("date: 2026-11-25\n"), "{text}");
    assert!(text.contains("review: 2027-02-23\n"), "{text}");
    assert!(text.contains("kind: rule\n"), "{text}");
}

#[test]
fn index_then_search_finds_the_added_note() {
    let h = Home::new();
    h.init();
    h.run(&[
        "add",
        "the gate code is behind the mailbox",
        "--title",
        "Gate",
    ]);
    h.run(&["index"]);
    let hits = h.json(&["search", "mailbox", "--json"]);
    let hits = hits["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "0-inbox/gate.md");
    assert_eq!(hits[0]["title"], "Gate");
    assert!(hits[0]["id"].as_str().is_some());
    let shown = h.run(&["show", hits[0]["id"].as_str().unwrap()]);
    assert!(shown.contains("the gate code is behind the mailbox"));
    assert_eq!(h.run(&["show", "0-inbox/gate.md"]), shown);
}

#[test]
fn shared_only_hides_private_and_inbox_notes() {
    let h = Home::new();
    h.init();
    h.put("kept.md", "quokka shared\n");
    h.put(
        "hidden.md",
        "---\nvisibility: private\n---\nquokka private\n",
    );
    h.put("private/p.md", "quokka folder\n");
    h.put("0-inbox/i.md", "quokka inbox\n");
    h.run(&["index"]);
    let all = h.json(&["search", "quokka", "--json"]);
    assert_eq!(all["hits"].as_array().unwrap().len(), 4);
    let shared = h.json(&["search", "quokka", "--json", "--shared-only"]);
    let paths: Vec<&str> = shared["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["kept.md"]);
}

#[test]
fn terminal_output_strips_control_characters() {
    let h = Home::new();
    h.init();
    h.put(
        "evil.md",
        "---\ntitle: Evil \u{1b}[31mred\u{1b}]0;pwned\u{7}\n---\nwombat \u{1b}[2J\u{9b}31m body\u{7}\ttabbed\n",
    );
    h.run(&["index"]);
    for args in [
        vec!["search", "wombat"],
        vec!["show", "evil.md"],
        vec!["status"],
    ] {
        let out = h.run(&args);
        assert!(
            !out.chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'),
            "{args:?} printed a control character: {out:?}"
        );
    }
    let shown = h.run(&["show", "evil.md"]);
    assert!(shown.contains("wombat"));
    assert!(shown.contains("\ttabbed"));
}

#[test]
fn search_before_init_says_how_to_start() {
    let h = Home::new();
    let out = h.brain().args(["search", "anything"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("brain init --me"), "{err}");
}

#[test]
fn a_usage_error_exits_2() {
    let h = Home::new();
    let out = h.brain().args(["search"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn status_counts_skips_by_reason() {
    let h = Home::new();
    h.init();
    h.put("ok.md", "fine\n");
    fs::write(h.vault.path().join("bin.md"), [0xff, 0xfe]).unwrap();
    h.run(&["index"]);
    let s = h.json(&["status", "--json"]);
    assert_eq!(s["notes"], 1);
    assert_eq!(s["pending"], 1);
    assert_eq!(s["skipped"]["not utf-8"], 1);
    assert_eq!(s["unidentified"], 0);
}

fn id_in(text: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap()
        .to_string()
}

#[test]
fn inbox_lists_oldest_first_with_facts() {
    let h = Home::new();
    h.init();
    h.run(&[
        "--now",
        "2026-11-20T00:00:00Z",
        "add",
        "older thought [unverified]",
        "--title",
        "Older",
    ]);
    h.run(&[
        "--now",
        "2026-11-24T00:00:00Z",
        "add",
        "newer thought",
        "--title",
        "Newer",
    ]);
    let out = h.run(&["inbox"]);
    let older = out.find("0-inbox/older.md").unwrap();
    let newer = out.find("0-inbox/newer.md").unwrap();
    assert!(older < newer, "{out}");
    assert!(out.contains("[unverified]"), "{out}");
}

#[test]
fn keep_moves_and_keeps_the_id() {
    let h = Home::new();
    h.init();
    h.run(&["add", "a thought worth keeping", "--title", "Keeper"]);
    let before = fs::read(h.vault.path().join("0-inbox/keeper.md")).unwrap();
    let id = id_in(&String::from_utf8_lossy(&before));
    h.run(&["inbox", "keep", &id, "--to", "permanent"]);
    assert!(!h.vault.path().join("0-inbox/keeper.md").exists());
    let after = fs::read(h.vault.path().join("3-permanent/keeper.md")).unwrap();
    assert_eq!(after, before, "keep changes no byte");
    h.run(&["add", "another", "--title", "Second"]);
    h.run(&["inbox", "keep", "0-inbox/second.md"]);
    assert!(h.vault.path().join("1-fleeting/second.md").exists());
}

#[test]
fn an_inbox_note_with_broken_frontmatter_is_listed_kept_and_skipped() {
    let h = Home::new();
    h.init();
    let broken = "---\ntitle: oops\nno closing fence\nwalrus here\n";
    h.put("0-inbox/broken.md", broken);
    h.put("0-inbox/also-broken.md", broken);
    let listed = h.run(&["inbox"]);
    assert!(listed.contains("0-inbox/broken.md"), "{listed}");
    assert!(listed.contains("frontmatter is broken"), "{listed}");
    h.run(&["inbox", "keep", "0-inbox/broken.md"]);
    assert_eq!(
        fs::read_to_string(h.vault.path().join("1-fleeting/broken.md")).unwrap(),
        broken
    );
    h.run(&["inbox", "skip", "0-inbox/also-broken.md"]);
    assert!(!h.vault.path().join("0-inbox/also-broken.md").exists());
}

#[test]
fn a_kept_note_can_be_shared() {
    let h = Home::new();
    h.init();
    h.run(&["add", "the numbat protocol", "--title", "Numbat"]);
    h.run(&["index"]);
    let shared = h.json(&["search", "numbat", "--json", "--shared-only"]);
    assert_eq!(
        shared["hits"].as_array().unwrap().len(),
        0,
        "an inbox note is never shared"
    );
    h.run(&["inbox", "keep", "0-inbox/numbat.md", "--to", "permanent"]);
    h.run(&["index"]);
    let shared = h.json(&["search", "numbat", "--json", "--shared-only"]);
    assert_eq!(shared["hits"][0]["path"], "3-permanent/numbat.md");
    assert_eq!(shared["hits"][0]["disclosure"], "shared");
}

#[test]
fn skip_archives_never_deletes() {
    let h = Home::new();
    h.init();
    h.run(&["add", "not worth keeping", "--title", "Skip me"]);
    let before = fs::read(h.vault.path().join("0-inbox/skip-me.md")).unwrap();
    let id = id_in(&String::from_utf8_lossy(&before));
    h.run(&["inbox", "skip", &id]);
    assert!(!h.vault.path().join("0-inbox/skip-me.md").exists());
    assert_eq!(
        fs::read(h.vault.path().join("_archive/0-inbox/skip-me.md")).unwrap(),
        before
    );
    let out = h.brain().args(["inbox", "skip", &id]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "a note no longer in the inbox");
}

/// Stamps `path` as last changed at `at`, and returns the date that is on this machine.
fn set_changed(path: &std::path::Path, at: &str) -> String {
    let at: jiff::Timestamp = at.parse().unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(at.into())
        .unwrap();
    at.to_zoned(jiff::tz::TimeZone::system()).date().to_string()
}

/// When `path` was last changed, to the second.
fn changed_at(path: &std::path::Path) -> i64 {
    let modified = std::fs::metadata(path).unwrap().modified().unwrap();
    jiff::Timestamp::try_from(modified).unwrap().as_second()
}
