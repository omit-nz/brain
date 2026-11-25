//! `brain daemon` as a child process: what it indexes, what it ignores, and how the other writers
//! defer to it. Each test waits on `brain status` or the daemon's `pass N:` lines, up to `LIMIT`.

use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::Home;
use super::import::link_dir;

const LIMIT: Duration = Duration::from_secs(20);

/// A running `brain daemon`, killed when dropped, with everything it printed to stderr.
struct Daemon {
    child: Child,
    stderr: Arc<Mutex<String>>,
}

impl Daemon {
    fn start(h: &Home) -> Daemon {
        Daemon::start_with(h, &[])
    }

    fn start_with(h: &Home, env: &[(&str, &str)]) -> Daemon {
        let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("brain"))
            .arg("daemon")
            .envs(env.iter().copied())
            .env("BRAIN_HOME", h.home.path())
            .env_remove("ORT_DYLIB_PATH")
            .env_remove("LL_MODELS_DIR")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = Arc::new(Mutex::new(String::new()));
        let sink = stderr.clone();
        let pipe = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                let mut s = sink.lock().unwrap();
                s.push_str(&line);
                s.push('\n');
            }
        });
        Daemon { child, stderr }
    }

    fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    fn passes(&self) -> usize {
        self.stderr()
            .lines()
            .filter(|l| l.starts_with("pass "))
            .count()
    }

    fn wait_for_passes(&self, n: usize) {
        until(
            &format!("pass {n}"),
            || self.passes() >= n,
            || self.stderr(),
        );
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.kill();
    }
}

fn until(what: &str, done: impl Fn() -> bool, log: impl Fn() -> String) {
    let deadline = Instant::now() + LIMIT;
    while !done() {
        assert!(
            Instant::now() < deadline,
            "no {what} within {LIMIT:?}:\n{}",
            log()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn notes(h: &Home) -> u64 {
    h.json(&["status", "--json"])["notes"].as_u64().unwrap()
}

fn wait_for_notes(h: &Home, d: &Daemon, n: u64) {
    until(&format!("{n} notes"), || notes(h) == n, || d.stderr());
}

fn note_with_id(n: usize, body: &str) -> String {
    format!("---\nid: 0195f2e0-0000-7000-8000-{n:012x}\n---\n{body}\n")
}

/// Paths of a search's hits, sorted.
fn found(h: &Home, query: &str) -> Vec<String> {
    let hits: Value = h.json(&["search", "--json", "--limit", "1000", query]);
    let mut paths: Vec<String> = hits
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["path"].as_str().unwrap().to_string())
        .collect();
    paths.sort();
    paths
}

#[test]
fn the_daemon_indexes_a_new_note() {
    let h = Home::new();
    h.init();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    h.put("a.md", &note_with_id(1, "a note about pelicans"));
    wait_for_notes(&h, &d, 1);
    assert_eq!(found(&h, "pelicans"), vec!["a.md"]);
}

#[test]
fn the_daemon_catches_up_after_a_burst_an_atomic_save_and_a_folder_rename() {
    let h = Home::new();
    h.init();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    for n in 0..500 {
        h.put(&format!("x/n{n:03}.md"), &note_with_id(n, "burst note"));
    }
    h.put("x/n001.md.tmp", &note_with_id(1, "an editor saved this"));
    fs::rename(
        h.vault.path().join("x/n001.md.tmp"),
        h.vault.path().join("x/n001.md"),
    )
    .unwrap();
    until(
        "the burst and the save",
        || notes(&h) == 500 && found(&h, "editor saved") == vec!["x/n001.md"],
        || d.stderr(),
    );
    // Once the burst's last events are spent, only the folder's own rename can wake the daemon.
    std::thread::sleep(Duration::from_secs(2));
    let before = d.passes();
    fs::rename(h.vault.path().join("x"), h.vault.path().join("y")).unwrap();
    d.wait_for_passes(before + 1);
    let fresh = Home {
        home: tempfile::tempdir().unwrap(),
        vault: tempfile::tempdir().unwrap(),
    };
    fresh.run(&[
        "init",
        "--me",
        "robin",
        "--vault",
        h.vault.path().to_str().unwrap(),
    ]);
    fresh.run(&["index"]);
    assert_eq!(notes(&fresh), 500);
    let queries = ["burst", "editor saved"];
    until(
        "the index a fresh `brain index` builds",
        || notes(&h) == 500 && queries.iter().all(|q| found(&h, q) == found(&fresh, q)),
        || d.stderr(),
    );
    assert_eq!(found(&h, "editor saved"), vec!["y/n001.md"]);
}

#[test]
fn hidden_folders_do_not_wake_it() {
    let h = Home::new();
    h.init();
    h.put("a.md", &note_with_id(1, "seeded"));
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    for n in 0..50 {
        h.put(&format!(".obsidian/w{n}.md"), "workspace");
        fs::rename(
            h.vault.path().join(format!(".obsidian/w{n}.md")),
            h.vault.path().join(format!(".obsidian/r{n}.md")),
        )
        .unwrap();
    }
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(d.passes(), 1, "{}", d.stderr());
    h.put("b.md", &note_with_id(2, "a visible note"));
    d.wait_for_passes(2);
}

#[test]
fn the_watcher_does_not_walk_a_linked_folder() {
    let h = Home::new();
    h.init();
    let elsewhere = tempfile::tempdir().unwrap();
    for n in 0..5000 {
        fs::write(elsewhere.path().join(format!("f{n}.md")), "elsewhere").unwrap();
    }
    link_dir(elsewhere.path(), &h.vault.path().join("linked"));
    let started = Instant::now();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "start-up took {:?}",
        started.elapsed()
    );
    assert_eq!(notes(&h), 0);
    fs::write(elsewhere.path().join("new.md"), "written through the link").unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(d.passes(), 1, "{}", d.stderr());
    h.put("a.md", &note_with_id(1, "a note in the vault"));
    d.wait_for_passes(2);
}

#[test]
fn an_id_is_not_written_into_a_note_saved_moments_ago() {
    let h = Home::new();
    h.init();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    let typed = "a note still being typed\n";
    h.put("typing.md", typed);
    d.wait_for_passes(2);
    assert_eq!(
        fs::read_to_string(h.vault.path().join("typing.md")).unwrap(),
        typed
    );
    until(
        "an id in the settled note",
        || {
            fs::read_to_string(h.vault.path().join("typing.md"))
                .unwrap()
                .starts_with("---\nid: ")
        },
        || d.stderr(),
    );
    wait_for_notes(&h, &d, 1);
}

#[test]
fn a_second_daemon_waits_for_the_first() {
    let h = Home::new();
    h.init();
    let mut first = Daemon::start(&h);
    first.wait_for_passes(1);
    let second = Daemon::start(&h);
    until(
        "the wait line",
        || second.stderr().contains("waiting for another brain writer"),
        || second.stderr(),
    );
    assert_eq!(second.passes(), 0);
    first.kill();
    second.wait_for_passes(1);
    h.put("a.md", &note_with_id(1, "after the hand-over"));
    wait_for_notes(&h, &second, 1);
}

#[test]
fn a_killed_daemon_leaves_no_lock_behind() {
    let h = Home::new();
    h.init();
    for n in 0..2000 {
        h.put(&format!("n{n:04}.md"), &note_with_id(n, "one of many"));
    }
    // The debug build's abort hook ends the process mid-index, as `kill -9` would.
    let mut first = Daemon::start_with(&h, &[("BRAIN_TEST_ABORT_AFTER_NOTES", "1000")]);
    let status = first.child.wait().unwrap();
    assert!(!status.success(), "{}", first.stderr());
    assert_eq!(first.passes(), 0);
    let second = Daemon::start(&h);
    second.wait_for_passes(1);
    assert!(!second.stderr().contains("waiting"), "{}", second.stderr());
    wait_for_notes(&h, &second, 2000);
}

#[test]
fn index_defers_to_a_running_daemon() {
    let h = Home::new();
    h.init();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    let out = h.run(&["index"]);
    assert!(
        out.contains("brain's daemon keeps the index up to date"),
        "{out}"
    );
    assert!(out.contains("or another brain command"), "{out}");
}

#[test]
fn a_status_probe_never_reads_as_a_daemon() {
    let h = Home::new();
    h.init();
    h.put("a.md", &note_with_id(1, "probed"));
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for _ in 0..50 {
                    h.run(&["status"]);
                }
            });
        }
        // One after another: two index runs at once rightly defer to each other.
        for _ in 0..20 {
            let out = h.run(&["index"]);
            assert!(!out.contains("daemon"), "{out}");
        }
    });
}

#[test]
fn import_leaves_indexing_to_a_running_daemon() {
    let h = Home::new();
    h.init();
    let src = tempfile::tempdir().unwrap();
    fs::write(src.path().join("one.md"), "first source note\n").unwrap();
    fs::write(
        src.path().join("two.md"),
        note_with_id(2, "second source note"),
    )
    .unwrap();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    let import = || {
        h.run(&[
            "import",
            "learning-loop",
            "--from",
            src.path().to_str().unwrap(),
        ])
    };
    let out = import();
    assert!(out.contains("brain's daemon will index them"), "{out}");
    wait_for_notes(&h, &d, 2);
    fs::write(src.path().join("one.md"), "first source note, edited\n").unwrap();
    let again = import();
    assert!(again.contains("0 new, 1 updated"), "{again}");
    assert!(again.contains("0 conflicts"), "{again}");
}

/// A folder of notes moved in from outside the vault: Windows reports only the folder's arrival.
#[test]
fn a_folder_of_notes_moved_in_is_indexed() {
    let h = Home::new();
    h.init();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    let outside = h.vault.path().with_extension("outside");
    fs::create_dir_all(&outside).unwrap();
    for n in 0..3 {
        fs::write(
            outside.join(format!("m{n}.md")),
            note_with_id(n, "moved in"),
        )
        .unwrap();
    }
    fs::rename(&outside, h.vault.path().join("moved")).unwrap();
    wait_for_notes(&h, &d, 3);
}

/// The daemon embeds a batch a turn and keeps turning while notes are pending, without waiting
/// for the next pass.
#[test]
fn the_daemon_embeds_every_pending_note_a_batch_at_a_time() {
    let h = Home::new();
    h.init();
    for n in 0..100 {
        h.put(&format!("n{n:03}.md"), &note_with_id(n, "to embed"));
    }
    let d = Daemon::start_with(&h, &[("BRAIN_TEST_EMBEDDER", "1")]);
    until(
        "100 embedded",
        || h.json(&["status", "--json"])["embedded"] == 100,
        || d.stderr(),
    );
    assert_eq!(d.passes(), 1, "{}", d.stderr());
}

/// The vault moved away and back (an unmounted disk, a rename undone): the failed pass is tried
/// again soon, and a note saved after the return is indexed.
#[test]
fn the_daemon_follows_its_vault_away_and_back() {
    let h = Home::new();
    h.init();
    h.put("a.md", &note_with_id(1, "before"));
    let d = Daemon::start(&h);
    wait_for_notes(&h, &d, 1);
    let away = h.vault.path().with_extension("away");
    fs::rename(h.vault.path(), &away).unwrap();
    until(
        "a failed pass",
        || d.stderr().contains("failed"),
        || d.stderr(),
    );
    fs::rename(&away, h.vault.path()).unwrap();
    assert_eq!(notes(&h), 1, "the index outlived the vault's absence");
    h.put("b.md", &note_with_id(2, "after"));
    wait_for_notes(&h, &d, 2);
}

#[test]
fn status_says_whether_the_daemon_runs() {
    let h = Home::new();
    h.init();
    assert_eq!(h.json(&["status", "--json"])["daemon"], false);
    assert!(h.run(&["status"]).contains("daemon: not running"));
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    assert_eq!(h.json(&["status", "--json"])["daemon"], true);
    assert!(h.run(&["status"]).contains("daemon: running"));
}
