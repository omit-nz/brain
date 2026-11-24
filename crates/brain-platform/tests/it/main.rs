use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use brain_platform::clock::{Clock, FixedClock};
use brain_platform::dirs::{Dirs, parse_brain_home};
use brain_platform::fs::{Privacy, archive, rename_within, write_atomic};

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
