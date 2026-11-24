use std::fs;
use std::path::Path;

use brain_note::{Entry, MAX_NOTE_BYTES, Place, Skip, Vault, rel_key};

/// macOS and Windows disks ignore case, so `Private/` is the private folder there; folding case
/// everywhere errs towards private on a disk that does not.
#[test]
fn a_layout_folder_in_another_case_keeps_its_place() {
    let root = tempfile::tempdir().unwrap();
    put(root.path(), "Private/salary.md", "pay\n");
    put(root.path(), "0-Inbox/draft.md", "draft\n");
    put(root.path(), "PRIVATE-notes/open.md", "open\n");
    let vault = Vault::open(root.path()).unwrap();
    let mut places: Vec<(String, Place)> = vault
        .walk()
        .filter_map(|e| match e {
            Entry::Note { rel, place } => Some((rel_key(&rel), place)),
            Entry::Skipped { .. } => None,
        })
        .collect();
    places.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        places,
        vec![
            ("0-Inbox/draft.md".to_string(), Place::Inbox),
            ("PRIVATE-notes/open.md".to_string(), Place::Kept),
            ("Private/salary.md".to_string(), Place::Private),
        ]
    );
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
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

fn notes(v: &Vault) -> Vec<(String, Place)> {
    v.walk()
        .filter_map(|e| match e {
            Entry::Note { rel, place } => Some((rel_key(&rel), place)),
            Entry::Skipped { .. } => None,
        })
        .collect()
}

fn skipped(v: &Vault) -> Vec<(String, Skip)> {
    v.walk()
        .filter_map(|e| match e {
            Entry::Skipped { rel, why } => Some((rel_key(&rel), why)),
            Entry::Note { .. } => None,
        })
        .collect()
}

fn keys(v: &Vault) -> Vec<String> {
    notes(v).into_iter().map(|(k, _)| k).collect()
}

#[test]
fn the_walk_takes_md_files_only() {
    let d = tempfile::tempdir().unwrap();
    for f in ["a.md", "b.txt", "c.MD", "d.markdown", "e/f.md", "g"] {
        put(d.path(), f, "x");
    }
    let v = Vault::open(d.path()).unwrap();
    assert_eq!(keys(&v), vec!["a.md", "e/f.md"]);
}

#[test]
fn dot_and_underscore_folders_are_not_walked() {
    let d = tempfile::tempdir().unwrap();
    for f in [
        ".obsidian/w.md",
        "_archive/a.md",
        "_system/s.md",
        ".hidden.md",
        "kept/.trash/x.md",
        "ok.md",
    ] {
        put(d.path(), f, "x");
    }
    let v = Vault::open(d.path()).unwrap();
    assert_eq!(keys(&v), vec!["ok.md"]);
    assert_eq!(skipped(&v), vec![]);
}

#[test]
fn the_walk_never_follows_a_link() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    put(outside.path(), "theirs.md", "x");
    put(d.path(), "mine.md", "x");
    link_dir(outside.path(), &d.path().join("linked"));
    link_dir(outside.path(), &d.path().join("looks.md"));
    let v = Vault::open(d.path()).unwrap();
    assert_eq!(keys(&v), vec!["mine.md"]);
    assert_eq!(
        skipped(&v),
        vec![
            ("linked".into(), Skip::Link),
            ("looks.md".into(), Skip::Link)
        ]
    );
    assert_eq!(v.read(Path::new("looks.md")), Err(Skip::Link));
    assert_eq!(v.read(Path::new("linked/theirs.md")), Err(Skip::Link));
}

#[test]
fn a_root_that_is_a_link_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    link_dir(outside.path(), &d.path().join("vault"));
    assert!(Vault::open(&d.path().join("vault")).is_err());
    assert!(Vault::open(&d.path().join("missing")).is_err());
}

#[test]
fn oversize_and_binary_files_are_skipped_with_a_reason() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "big.md", &"x".repeat(MAX_NOTE_BYTES as usize + 1));
    put(d.path(), "edge.md", &"y".repeat(MAX_NOTE_BYTES as usize));
    fs::write(d.path().join("bin.md"), [0xff, 0xfe, 0x00, 0x80]).unwrap();
    let v = Vault::open(d.path()).unwrap();
    assert_eq!(keys(&v), vec!["bin.md", "edge.md"]);
    assert_eq!(
        skipped(&v),
        vec![("big.md".into(), Skip::TooLarge(MAX_NOTE_BYTES + 1))]
    );
    assert_eq!(v.read(Path::new("bin.md")), Err(Skip::NotUtf8));
    assert_eq!(
        v.read(Path::new("big.md")),
        Err(Skip::TooLarge(MAX_NOTE_BYTES + 1))
    );
    assert_eq!(
        v.read(Path::new("edge.md")).unwrap().len(),
        MAX_NOTE_BYTES as usize
    );
}

/// Holds a folder so the walk cannot list it: mode 000 on Unix, an open handle that shares
/// nothing on Windows.
struct Locked {
    #[cfg(unix)]
    path: std::path::PathBuf,
    #[cfg(windows)]
    _handle: fs::File,
}

impl Locked {
    fn new(path: &Path) -> Locked {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
            Locked {
                path: path.to_path_buf(),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            let handle = fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .unwrap();
            Locked { _handle: handle }
        }
    }
}

#[cfg(unix)]
impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o755));
    }
}

#[test]
fn an_unreadable_folder_is_skipped_and_the_walk_goes_on() {
    let d = tempfile::tempdir().unwrap();
    for f in ["a.md", "locked/x.md", "z/b.md"] {
        put(d.path(), f, "x");
    }
    let v = Vault::open(d.path()).unwrap();
    let _lock = Locked::new(&d.path().join("locked"));
    assert_eq!(keys(&v), vec!["a.md", "z/b.md"]);
    let s = skipped(&v);
    assert_eq!(s.len(), 1, "{s:?}");
    assert_eq!(s[0].0, "locked");
    assert!(matches!(s[0].1, Skip::Unreadable(_)), "{s:?}");
}

#[test]
fn a_folder_named_x_md_is_walked_into() {
    let d = tempfile::tempdir().unwrap();
    put(d.path(), "x.md/inner.md", "x");
    let v = Vault::open(d.path()).unwrap();
    assert_eq!(keys(&v), vec!["x.md/inner.md"]);
}

#[test]
fn places_follow_the_layout() {
    let d = tempfile::tempdir().unwrap();
    for f in [
        "0-inbox/i.md",
        "private/p.md",
        "working-memory/w.md",
        "3-permanent/k.md",
        "inbox.md",
    ] {
        put(d.path(), f, "x");
    }
    let v = Vault::open(d.path()).unwrap();
    assert_eq!(
        notes(&v),
        vec![
            ("0-inbox/i.md".into(), Place::Inbox),
            ("3-permanent/k.md".into(), Place::Kept),
            ("inbox.md".into(), Place::Kept),
            ("private/p.md".into(), Place::Private),
            ("working-memory/w.md".into(), Place::Kept),
        ]
    );
}

#[test]
fn the_walk_order_is_stable() {
    let d = tempfile::tempdir().unwrap();
    for f in ["b.md", "a/z.md", "C.md", "a.md", "c/a.md"] {
        put(d.path(), f, "x");
    }
    let v = Vault::open(d.path()).unwrap();
    let first = keys(&v);
    assert_eq!(first, vec!["C.md", "a/z.md", "a.md", "b.md", "c/a.md"]);
    assert_eq!(keys(&v), first);
}
