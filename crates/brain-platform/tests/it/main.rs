use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use brain_platform::clock::{Clock, FixedClock};
use brain_platform::dirs::{Dirs, parse_brain_home};
use brain_platform::fs::{
    Lock, Privacy, archive, create_dir_private, create_new, ensure_private_file, rename_within,
    replace_unchanged, write_atomic,
};

#[test]
fn create_new_never_replaces_a_file() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("inbox/note.md");
    create_new(root.path(), rel, b"first", Privacy::Normal).unwrap();
    let err = create_new(root.path(), rel, b"second", Privacy::Normal).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert_eq!(fs::read(root.path().join(rel)).unwrap(), b"first");
    let leftovers = fs::read_dir(root.path().join("inbox")).unwrap().count();
    assert_eq!(leftovers, 1, "no temp file is left behind");
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("x")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(elsewhere.path(), root.path().join("linked")).unwrap();
    #[cfg(unix)]
    assert!(create_new(root.path(), Path::new("linked/n.md"), b"x", Privacy::Normal).is_err());
}
use brain_platform::service::plist;

#[test]
fn a_replace_refuses_a_file_changed_since_it_was_read() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("n.md");
    fs::write(root.path().join(rel), "edited\n").unwrap();
    let err = replace_unchanged(
        root.path(),
        rel,
        |now| now == Some(b"read\n".as_slice()),
        b"new\n",
        Privacy::Normal,
    )
    .unwrap_err();
    assert!(err.to_string().contains("changed since"), "{err}");
    assert_eq!(
        fs::read_to_string(root.path().join(rel)).unwrap(),
        "edited\n"
    );

    replace_unchanged(
        root.path(),
        rel,
        |now| now == Some(b"edited\n".as_slice()),
        b"new\n",
        Privacy::Normal,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(root.path().join(rel)).unwrap(), "new\n");

    let fresh = Path::new("fresh.md");
    replace_unchanged(
        root.path(),
        fresh,
        |now| now.is_none(),
        b"first\n",
        Privacy::Normal,
    )
    .unwrap();
    assert!(
        replace_unchanged(
            root.path(),
            fresh,
            |now| now.is_none(),
            b"second\n",
            Privacy::Normal
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(root.path().join(fresh)).unwrap(),
        "first\n"
    );
}

#[test]
fn ensure_private_file_creates_once_and_tightens() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("data/index.db");
    ensure_private_file(root.path(), rel).unwrap();
    let path = root.path().join(rel);
    assert_eq!(fs::read(&path).unwrap(), b"");

    fs::write(&path, b"kept").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    ensure_private_file(root.path(), rel).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        b"kept",
        "an existing file is never replaced"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let outside = tempfile::tempdir().unwrap();
    link_dir(outside.path(), &root.path().join("linked"));
    let err = ensure_private_file(root.path(), Path::new("linked/x.db")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn racing_creators_share_one_file() {
    let root = tempfile::tempdir().unwrap();
    let rel = Path::new("index.db");
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| ensure_private_file(root.path(), rel).unwrap());
        }
    });
    let names: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec!["index.db"]);
}

#[test]
fn from_home_holds_every_dir() {
    let home = tempfile::tempdir().unwrap();
    let d = Dirs::from_home(home.path());
    for p in [&d.config, &d.data, &d.models, &d.bin] {
        assert!(
            p.starts_with(home.path()),
            "{} is outside BRAIN_HOME",
            p.display()
        );
    }
    assert_eq!(d.models, d.data.join("models"));
    assert_eq!(d.bin, d.data.join("bin"));
}

#[test]
fn a_relative_brain_home_is_refused() {
    assert!(parse_brain_home("relative/home").is_err());
    let abs = tempfile::tempdir().unwrap();
    assert_eq!(
        parse_brain_home(abs.path().to_str().unwrap()).unwrap(),
        abs.path()
    );
}

#[test]
fn write_atomic_leaves_no_temp_file_on_success() {
    let root = tempfile::tempdir().unwrap();
    write_atomic(root.path(), Path::new("a/b.md"), b"hello", Privacy::Normal).unwrap();
    assert_eq!(fs::read(root.path().join("a/b.md")).unwrap(), b"hello");
    let names: Vec<_> = fs::read_dir(root.path().join("a"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec!["b.md"]);
}

#[test]
fn a_private_file_is_owner_only() {
    let root = tempfile::tempdir().unwrap();
    write_atomic(root.path(), Path::new("secret.db"), b"x", Privacy::Private).unwrap();
    let path = root.path().join("secret.db");
    assert_eq!(fs::read(&path).unwrap(), b"x");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
    }
}

#[test]
fn a_normal_file_follows_the_umask() {
    let root = tempfile::tempdir().unwrap();
    write_atomic(root.path(), Path::new("note.md"), b"x", Privacy::Normal).unwrap();
    fs::write(root.path().join("plain.md"), b"x").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |n: &str| {
            fs::metadata(root.path().join(n))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("note.md"), mode("plain.md"));
    }
}

fn link_dir(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let ok = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()
            .unwrap()
            .success();
        assert!(ok, "mklink /J failed");
    }
}

#[test]
fn a_write_through_a_linked_folder_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    link_dir(outside.path(), &root.path().join("linked"));
    let err =
        write_atomic(root.path(), Path::new("linked/x.md"), b"x", Privacy::Normal).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(!outside.path().join("x.md").exists());
}

#[test]
fn a_write_to_a_linked_final_component_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    link_dir(outside.path(), &root.path().join("linked.md"));
    let err = write_atomic(root.path(), Path::new("linked.md"), b"x", Privacy::Normal).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn paths_that_leave_the_root_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let inner = root.path().join("vault");
    write_atomic(&inner, Path::new("a.md"), b"a", Privacy::Normal).unwrap();
    let escapes = [Path::new("../x.md"), Path::new("a/../../x.md"), root.path()];
    for rel in escapes {
        let w = write_atomic(&inner, rel, b"x", Privacy::Normal).unwrap_err();
        assert_eq!(w.kind(), ErrorKind::InvalidInput, "write {}", rel.display());
        let r = rename_within(&inner, Path::new("a.md"), rel).unwrap_err();
        assert_eq!(
            r.kind(),
            ErrorKind::InvalidInput,
            "rename to {}",
            rel.display()
        );
    }
    assert!(!root.path().join("x.md").exists());
    assert_eq!(fs::read(inner.join("a.md")).unwrap(), b"a");
}

#[test]
fn rename_within_refuses_a_link_on_either_side() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    link_dir(outside.path(), &root.path().join("linked"));
    write_atomic(root.path(), Path::new("a.md"), b"a", Privacy::Normal).unwrap();
    let to = rename_within(root.path(), Path::new("a.md"), Path::new("linked/a.md")).unwrap_err();
    assert_eq!(to.kind(), ErrorKind::InvalidInput);
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    assert!(root.path().join("a.md").exists());

    fs::write(outside.path().join("x.md"), b"theirs").unwrap();
    let from = rename_within(root.path(), Path::new("linked/x.md"), Path::new("x.md")).unwrap_err();
    assert_eq!(from.kind(), ErrorKind::InvalidInput);
    assert_eq!(fs::read(outside.path().join("x.md")).unwrap(), b"theirs");
    assert!(!root.path().join("x.md").exists());
}

#[test]
fn a_failed_rename_leaves_no_new_folders() {
    let root = tempfile::tempdir().unwrap();
    let err = rename_within(
        root.path(),
        Path::new("missing.md"),
        Path::new("new/deep/x.md"),
    )
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(!root.path().join("new").exists());
}

#[test]
fn a_private_dir_is_owner_only_and_refuses_a_link() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("a/b");
    create_dir_private(&dir).unwrap();
    assert!(dir.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [&dir, &root.path().join("a")] {
            let mode = fs::metadata(p).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "{} is {mode:o}", p.display());
        }
    }
    let outside = tempfile::tempdir().unwrap();
    let linked = root.path().join("linked");
    link_dir(outside.path(), &linked);
    let err = create_dir_private(&linked).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[cfg(unix)]
#[test]
fn rewriting_a_note_keeps_the_mode_its_owner_set() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("note.md");
    write_atomic(root.path(), Path::new("note.md"), b"one", Privacy::Normal).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    write_atomic(root.path(), Path::new("note.md"), b"two", Privacy::Normal).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"two");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

/// The index and models are large and local, so on Windows they live under Local, never in a
/// roaming profile that syncs at sign-in.
#[cfg(windows)]
#[test]
fn data_is_local_not_roaming() {
    if std::env::var_os("BRAIN_HOME").is_some() {
        return;
    }
    let d = Dirs::resolve().unwrap();
    let data = d.data.to_string_lossy().to_lowercase();
    assert!(!data.contains("roaming"), "{data}");
}

#[test]
fn rename_within_refuses_an_existing_target() {
    let root = tempfile::tempdir().unwrap();
    write_atomic(root.path(), Path::new("a.md"), b"a", Privacy::Normal).unwrap();
    write_atomic(root.path(), Path::new("b.md"), b"b", Privacy::Normal).unwrap();
    let err = rename_within(root.path(), Path::new("a.md"), Path::new("b.md")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);
    assert_eq!(fs::read(root.path().join("b.md")).unwrap(), b"b");
    rename_within(root.path(), Path::new("a.md"), Path::new("new/a.md")).unwrap();
    assert_eq!(fs::read(root.path().join("new/a.md")).unwrap(), b"a");
    assert!(!root.path().join("a.md").exists());
}

#[test]
fn archive_moves_and_never_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let arch = Path::new("_archive");
    write_atomic(root.path(), Path::new("in/x.md"), b"one", Privacy::Normal).unwrap();
    let first = archive(root.path(), Path::new("in/x.md"), arch).unwrap();
    write_atomic(root.path(), Path::new("in/x.md"), b"two", Privacy::Normal).unwrap();
    let second = archive(root.path(), Path::new("in/x.md"), arch).unwrap();
    assert_eq!(first, Path::new("_archive/in/x.md"));
    assert_eq!(second, Path::new("_archive/in/x-1.md"));
    assert_eq!(fs::read(root.path().join(&first)).unwrap(), b"one");
    assert_eq!(fs::read(root.path().join(&second)).unwrap(), b"two");
    assert!(!root.path().join("in/x.md").exists());
}

#[test]
fn fixed_clock_is_fixed() {
    let t: jiff::Timestamp = "2026-11-25T00:00:00Z".parse().unwrap();
    let c = FixedClock(t);
    assert_eq!(c.now(), t);
    assert_eq!(c.now(), c.now());
    assert_eq!(c.today(), jiff::civil::date(2026, 11, 25));
}

/// Run by `Holder`: takes the lock named by `BRAIN_TEST_LOCK`, says so, and holds it until killed.
#[test]
#[ignore]
fn lock_holder_child() {
    let path = std::env::var_os("BRAIN_TEST_LOCK").unwrap();
    let _held = Lock::acquire(Path::new(&path)).unwrap();
    println!("held");
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

/// Another process holding the lock, until dropped or killed.
struct Holder(std::process::Child);

impl Holder {
    fn start(lock: &Path) -> Holder {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["lock_holder_child", "--exact", "--ignored", "--nocapture"])
            .env("BRAIN_TEST_LOCK", lock)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        while line.trim() != "held" {
            line.clear();
            assert_ne!(
                std::io::BufRead::read_line(&mut out, &mut line).unwrap(),
                0,
                "the holder exited before taking the lock"
            );
        }
        Holder(child)
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_lock_is_exclusive_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join("writer.lock");
    assert!(!Lock::is_held(&lock));
    let _holder = Holder::start(&lock);
    assert!(Lock::is_held(&lock));
    assert!(Lock::try_acquire(&lock).unwrap().is_none());
}

#[test]
fn a_killed_holder_frees_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join("writer.lock");
    let mut holder = Holder::start(&lock);
    holder.0.kill().unwrap();
    holder.0.wait().unwrap();
    // Windows releases a dead process's lock "depending upon available system resources".
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let taken = loop {
        if let Some(l) = Lock::try_acquire(&lock).unwrap() {
            break l;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "still held 10 s after its holder died"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    drop(taken);
    assert!(!Lock::is_held(&lock));
}

#[test]
fn a_probe_never_takes_the_lock_from_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let lock = dir.path().join("writer.lock");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let probers: Vec<_> = (0..4)
        .map(|_| {
            let (lock, stop) = (lock.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    Lock::is_held(&lock);
                }
            })
        })
        .collect();
    let refused = (0..500)
        .filter(|_| Lock::try_acquire(&lock).unwrap().is_none())
        .count();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for p in probers {
        p.join().unwrap();
    }
    assert_eq!(refused, 0, "a probe was read as a writer");
}

/// The element after `<key>{key}</key>` in a top-level dict, as its raw XML up to the next key
/// or the dict's end.
fn plist_value<'a>(plist: &'a str, key: &str) -> &'a str {
    let open = format!("<key>{key}</key>");
    let at = plist
        .find(&open)
        .unwrap_or_else(|| panic!("no {key}:\n{plist}"))
        + open.len();
    let rest = &plist[at..];
    let end = ["<key>", "\n</dict>"]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
    rest[..end].trim()
}

#[test]
fn the_plist_runs_the_daemon_and_restarts_it_on_a_crash() {
    let p = plist(
        Path::new("/Users/r/brain-current/brain"),
        Path::new("/Users/r/Library/Logs/brain/daemon.log"),
        None,
    );
    assert!(p.starts_with("<?xml"));
    assert_eq!(plist_value(&p, "Label"), "<string>nz.omit.brain</string>");
    assert_eq!(
        plist_value(&p, "ProgramArguments")
            .split_whitespace()
            .collect::<Vec<_>>(),
        [
            "<array>",
            "<string>/Users/r/brain-current/brain</string>",
            "<string>daemon</string>",
            "</array>"
        ]
    );
    assert_eq!(plist_value(&p, "RunAtLoad"), "<true/>");
    let flat = p.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("<key>KeepAlive</key> <dict> <key>SuccessfulExit</key> <false/> </dict>"),
        "{p}"
    );
    assert_eq!(
        plist_value(&p, "ProcessType"),
        "<string>Background</string>"
    );
    for key in ["StandardErrorPath", "StandardOutPath"] {
        assert_eq!(
            plist_value(&p, key),
            "<string>/Users/r/Library/Logs/brain/daemon.log</string>"
        );
    }
    assert!(!p.contains("EnvironmentVariables"));
    let with_home = plist(Path::new("/b"), Path::new("/l"), Some(Path::new("/h")));
    let flat = with_home.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains(
            "<key>EnvironmentVariables</key> <dict> <key>BRAIN_HOME</key> <string>/h</string> </dict>"
        ),
        "{with_home}"
    );
    #[cfg(target_os = "macos")]
    for text in [&p, &with_home] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agent.plist");
        fs::write(&file, text).unwrap();
        let lint = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            lint.status.success(),
            "{}",
            String::from_utf8_lossy(&lint.stdout)
        );
    }
}

#[test]
fn a_path_with_xml_characters_is_escaped() {
    let p = plist(
        Path::new("/Users/a&b/<x>/\"q\"/brain"),
        Path::new("/l"),
        Some(Path::new("/h&<")),
    );
    assert!(p.contains("<string>/Users/a&amp;b/&lt;x&gt;/&quot;q&quot;/brain</string>"));
    assert!(p.contains("<string>/h&amp;&lt;</string>"));
    assert!(!p.contains("a&b"));
}
