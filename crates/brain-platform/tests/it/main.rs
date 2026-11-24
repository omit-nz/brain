use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use brain_platform::clock::{Clock, FixedClock};
use brain_platform::dirs::{Dirs, parse_brain_home};
use brain_platform::fs::{
    Privacy, archive, create_dir_private, ensure_private_file, rename_within, replace_unchanged,
    write_atomic,
};

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
