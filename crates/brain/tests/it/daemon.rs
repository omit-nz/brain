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
    let mut paths: Vec<String> = hits["hits"]
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

/// Each recheck searches every open gap, so saves do not trigger one each: the first pass that
/// changed notes does, then nothing until an hour has gone.
#[test]
fn recheck_runs_at_most_hourly() {
    let h = Home::new();
    h.init();
    let d = Daemon::start(&h);
    d.wait_for_passes(1);
    for n in 0..20 {
        h.put(&format!("s{n}.md"), &note_with_id(n, "a save"));
        std::thread::sleep(Duration::from_millis(600));
    }
    wait_for_notes(&h, &d, 20);
    let rechecks = d
        .stderr()
        .lines()
        .filter(|l| l.starts_with("recheck"))
        .count();
    assert!(d.passes() > 2, "{}", d.stderr());
    assert_eq!(rechecks, 1, "{}", d.stderr());
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

/// The vault moved away and back (an unmounted disk, a rename undone): the index outlives the
/// absence, and a note saved after the return is indexed. macOS and Linux report the root going
/// and the pass fails and is retried with the watch set up again; Windows reports nothing, and
/// its watch follows the folder.
#[test]
fn the_daemon_follows_its_vault_away_and_back() {
    let h = Home::new();
    h.init();
    h.put("a.md", &note_with_id(1, "before"));
    let d = Daemon::start(&h);
    wait_for_notes(&h, &d, 1);
    let away = h.vault.path().with_extension("away");
    fs::rename(h.vault.path(), &away).unwrap();
    std::thread::sleep(Duration::from_secs(2));
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

// The daemon's own updates. Each home's install holds a copy of this build, so a daemon run from
// there is the install; the releases are signed locally and trusted through BRAIN_TEST_RELEASE_KEY.

/// A home whose `brain-current` holds a copy of this build; returns the copy's path.
fn installed_home() -> (Home, std::path::PathBuf) {
    let h = Home::new();
    h.init();
    let current = h.home.path().join("data/bin/brain-current");
    std::fs::create_dir_all(current.join("lib")).unwrap();
    let exe = current.join(brain::update::exe_name());
    std::fs::copy(assert_cmd::cargo::cargo_bin("brain"), &exe).unwrap();
    (h, exe)
}

/// A release folder with 9.9.9 signed by `signer`, and the hex of the key the daemon trusts. The
/// release's brain writes the arguments it was started with to `started` in the brain home.
fn releases(signer: u8) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    super::update::archive_holding(
        dir.path(),
        "9.9.9",
        &[super::update::key(signer)],
        "#!/bin/sh\necho \"brain 9.9.9 $*\" > \"$BRAIN_HOME/started\"\n",
    );
    let trusted = super::update::public(&super::update::key(7));
    (dir, trusted.iter().map(|b| format!("{b:02x}")).collect())
}

/// `exe daemon --from-dir releases`, its stderr in the home's `daemon.err`.
fn updating_daemon(
    h: &Home,
    exe: &std::path::Path,
    releases: &std::path::Path,
    hex: &str,
) -> std::process::Child {
    std::process::Command::new(exe)
        .args(["daemon", "--from-dir", releases.to_str().unwrap()])
        .env("BRAIN_HOME", h.home.path())
        .env("BRAIN_TEST_RELEASE_KEY", hex)
        .env_remove("ORT_DYLIB_PATH")
        .env_remove("LL_MODELS_DIR")
        .stderr(std::fs::File::create(h.home.path().join("daemon.err")).unwrap())
        .spawn()
        .unwrap()
}

/// Waits up to 60 s for `line` in the daemon's stderr, and returns all of it.
fn stderr_with(h: &Home, line: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let err = std::fs::read_to_string(h.home.path().join("daemon.err")).unwrap_or_default();
        if err.contains(line) {
            return err;
        }
        assert!(std::time::Instant::now() < deadline, "no {line:?} in {err}");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

// Replacing the folder a running binary lives in is a Unix move. The daemon starts the release it
// installed in its own place, so a daemon run in a terminal keeps running as one under launchd does.
#[cfg(unix)]
#[test]
fn the_daemon_installs_a_newer_release_and_runs_it_in_its_place() {
    let (h, exe) = installed_home();
    let (dir, hex) = releases(7);
    let d = updating_daemon(&h, &exe, dir.path(), &hex);
    assert_runs_9_9_9(&h, d, dir.path());
}

/// Waits for the daemon `d` to install 9.9.9 from `releases` and become it, in the same process
/// and with the same arguments.
#[cfg(unix)]
fn assert_runs_9_9_9(h: &Home, mut d: std::process::Child, releases: &std::path::Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status = loop {
        if let Some(s) = d.try_wait().unwrap() {
            break s;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the daemon never exited"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    stderr_with(h, "installed brain 9.9.9; starting it");
    assert!(status.success(), "{status}");
    assert_eq!(
        std::fs::read_to_string(h.home.path().join("started")).unwrap(),
        format!("brain 9.9.9 daemon --from-dir {}\n", releases.display())
    );
}

#[test]
fn the_daemon_refuses_an_unsigned_release_and_keeps_running() {
    let (h, exe) = installed_home();
    let before = std::fs::read(&exe).unwrap();
    let (dir, hex) = releases(8);
    let mut d = updating_daemon(&h, &exe, dir.path(), &hex);
    let err = stderr_with(&h, "update failed");
    assert!(err.to_lowercase().contains("signature"), "{err}");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let alive = d.try_wait().unwrap().is_none();
    let _ = d.kill();
    let _ = d.wait();
    assert!(alive, "the daemon keeps running the old version");
    assert!(
        std::fs::read(&exe).unwrap() == before,
        "the install is unchanged"
    );
}

#[test]
fn a_daemon_outside_the_install_never_updates() {
    let (h, exe) = installed_home();
    let before = std::fs::read(&exe).unwrap();
    let (dir, hex) = releases(7);
    let mut d = updating_daemon(&h, &assert_cmd::cargo::cargo_bin("brain"), dir.path(), &hex);
    stderr_with(&h, "not run from its install, so it never updates itself");
    std::thread::sleep(std::time::Duration::from_millis(500));
    let alive = d.try_wait().unwrap().is_none();
    let _ = d.kill();
    let _ = d.wait();
    assert!(alive);
    assert!(std::fs::read(&exe).unwrap() == before);
}

/// Saves a note and waits for the pass it wakes, `pass N:` in the daemon's stderr.
fn wake_pass(h: &Home, n: u32) {
    h.put(&format!("wake-{n}.md"), "a save that wakes a pass\n");
    stderr_with(h, &format!("pass {n}:"));
}

#[test]
fn a_failed_update_is_counted_and_tried_once_a_day_not_once_a_pass() {
    let (h, exe) = installed_home();
    let (dir, hex) = releases(8);
    let mut d = updating_daemon(&h, &exe, dir.path(), &hex);
    stderr_with(&h, "update failed");
    wake_pass(&h, 2);
    wake_pass(&h, 3);
    let err = std::fs::read_to_string(h.home.path().join("daemon.err")).unwrap();
    let _ = d.kill();
    let _ = d.wait();
    assert_eq!(err.matches("update failed").count(), 1, "{err}");
    let usage = h.run(&["usage"]);
    assert!(
        usage
            .lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["error_update", "-", "1"]),
        "{usage}"
    );
}

#[test]
fn a_daemon_outside_the_install_says_so_once() {
    let (h, _) = installed_home();
    let (dir, hex) = releases(7);
    let mut d = updating_daemon(&h, &assert_cmd::cargo::cargo_bin("brain"), dir.path(), &hex);
    stderr_with(&h, "never updates itself");
    wake_pass(&h, 2);
    let err = std::fs::read_to_string(h.home.path().join("daemon.err")).unwrap();
    let _ = d.kill();
    let _ = d.wait();
    assert_eq!(err.matches("never updates itself").count(), 1, "{err}");
}

#[cfg(unix)]
#[test]
fn a_daemon_started_through_a_link_into_the_install_still_updates() {
    let (h, exe) = installed_home();
    let link_dir = tempfile::tempdir().unwrap();
    let link = link_dir.path().join("brain");
    std::os::unix::fs::symlink(&exe, &link).unwrap();
    let (dir, hex) = releases(7);
    let d = updating_daemon(&h, &link, dir.path(), &hex);
    assert_runs_9_9_9(&h, d, dir.path());
}

/// A vault on a disk not yet mounted at login: the daemon waits for it rather than exiting into
/// a supervisor's restart loop.
#[test]
fn a_daemon_started_before_its_vault_waits_for_it() {
    let h = Home::new();
    h.init();
    h.put("a.md", &note_with_id(1, "waited"));
    let away = h.vault.path().with_extension("away");
    fs::rename(h.vault.path(), &away).unwrap();
    let mut d = Daemon::start(&h);
    until(
        "the waiting line",
        || d.stderr().contains("waiting for it"),
        || d.stderr(),
    );
    assert!(d.child.try_wait().unwrap().is_none(), "{}", d.stderr());
    fs::rename(&away, h.vault.path()).unwrap();
    wait_for_notes(&h, &d, 1);
    assert_eq!(
        d.stderr().matches("waiting for it").count(),
        1,
        "{}",
        d.stderr()
    );
}
