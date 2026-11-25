//! Every file write and move brain makes. Nothing here deletes: taking a file away is a move into
//! the archive, so nothing is lost without its owner saying so.

use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

pub enum Privacy {
    /// Owner-only (`0600` on Unix): stores, ledgers, keys. On Windows the file takes the ACL of
    /// its folder, which is owner-only under the person's profile and not promised elsewhere.
    Private,
    /// Whatever the process umask allows for a new file; a file that exists keeps its mode, so a
    /// note its owner locked down stays that way.
    Normal,
}

/// A symlink, or on Windows a junction or other name-surrogate reparse point; std's `is_symlink`
/// covers both. Cloud placeholders (OneDrive, iCloud) are reparse points too but are not links.
pub fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Refuses a relative path that leaves `root` or passes through a link on the way, so a vault
/// another app can write cannot redirect a write elsewhere.
fn checked(root: &Path, rel: &Path) -> io::Result<PathBuf> {
    let mut at = root.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(part) => {
                at.push(part);
                if is_link(&at) {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        format!("{} is a link", at.display()),
                    ));
                }
            }
            _ => {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("{} leaves its root", rel.display()),
                ));
            }
        }
    }
    Ok(at)
}

fn parent_dirs(path: &Path) -> io::Result<&Path> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "no parent"))?;
    fs::create_dir_all(parent)?;
    Ok(parent)
}

/// `write_atomic`, but only when `unchanged` accepts the file as it is now (`None` when there is
/// none), so brain never writes over a save made since it read the file. The check and the rename
/// are two steps; the window between them is what a file system allows without a lock.
pub fn replace_unchanged(
    root: &Path,
    rel: &Path,
    unchanged: impl FnOnce(Option<&[u8]>) -> bool,
    bytes: &[u8],
    privacy: Privacy,
) -> io::Result<()> {
    let now = match fs::read(checked(root, rel)?) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if !unchanged(now.as_deref()) {
        return Err(io::Error::other(format!(
            "{} changed since it was read; nothing written",
            rel.display()
        )));
    }
    write_atomic(root, rel, bytes, privacy)
}

pub fn write_atomic(root: &Path, rel: &Path, bytes: &[u8], privacy: Privacy) -> io::Result<()> {
    let path = checked(root, rel)?;
    let existing = match privacy {
        Privacy::Normal => fs::metadata(&path).ok().map(|m| m.permissions()),
        Privacy::Private => None,
    };
    let parent = parent_dirs(&path)?;
    let mut tmp = temp_in(parent, privacy)?;
    tmp.write_all(bytes)?;
    // Set on the open file, after the write: the create mode is masked by the umask, this is not.
    if let Some(p) = existing {
        tmp.as_file().set_permissions(p)?;
    }
    tmp.as_file().sync_all()?;
    tmp.persist(&path).map_err(|e| e.error)?;
    Ok(())
}

/// Writes a new file at `rel`, whole or not at all, and never over one that exists: a file that
/// appeared at `rel` first is `AlreadyExists`, so two writers choosing one name both keep their
/// bytes once the second picks another.
pub fn create_new(root: &Path, rel: &Path, bytes: &[u8], privacy: Privacy) -> io::Result<()> {
    let path = checked(root, rel)?;
    let mut tmp = temp_in(parent_dirs(&path)?, privacy)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(&path).map_err(|e| e.error)?;
    Ok(())
}

/// A temporary file beside its target, created with the mode `privacy` asks for.
fn temp_in(parent: &Path, privacy: Privacy) -> io::Result<tempfile::NamedTempFile> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(".brain-");
    if let Some(p) = permissions(privacy) {
        builder.permissions(p);
    }
    builder.tempfile_in(parent)
}

#[cfg(unix)]
fn permissions(privacy: Privacy) -> Option<fs::Permissions> {
    use std::os::unix::fs::PermissionsExt;
    // open(2) masks the mode with the umask, so Normal ends up as a plain file would.
    Some(fs::Permissions::from_mode(match privacy {
        Privacy::Private => 0o600,
        Privacy::Normal => 0o666,
    }))
}

/// Windows files inherit the ACL of their folder; see `Privacy::Private`.
#[cfg(not(unix))]
fn permissions(_: Privacy) -> Option<fs::Permissions> {
    None
}

/// Makes `rel` an owner-only file: created empty when absent, never replacing one that exists (so
/// processes creating it at once all end up with the same file), and an existing one's mode
/// tightened to `0600` on Unix.
pub fn ensure_private_file(root: &Path, rel: &Path) -> io::Result<()> {
    let path = checked(root, rel)?;
    let parent = parent_dirs(&path)?;
    let tmp = temp_in(parent, Privacy::Private)?;
    match tmp.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(e) if e.error.kind() == ErrorKind::AlreadyExists => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        }
        Err(e) => Err(e.error),
    }
}

/// Creates `path` and any missing parents owner-only (`0700` on Unix) from the start, and makes an
/// existing `path` owner-only. Refuses a `path` that is a link. The check and the create are two
/// steps, so a process racing to plant a link between them wins; brain's own folders are not
/// shared with other users, so that race is accepted.
pub fn create_dir_private(path: &Path) -> io::Result<()> {
    if is_link(path) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} is a link", path.display()),
        ));
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    builder.create(path)?;
    Ok(())
}

pub fn rename_within(root: &Path, from: &Path, to: &Path) -> io::Result<()> {
    let src = checked(root, from)?;
    let dst = checked(root, to)?;
    fs::symlink_metadata(&src)?;
    if fs::symlink_metadata(&dst).is_ok() {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            format!("{} exists", dst.display()),
        ));
    }
    parent_dirs(&dst)?;
    fs::rename(src, dst)
}

/// Moves `rel` under `archive_dir`, keeping its relative path and adding `-1`, `-2` on a clash.
/// Returns the new path relative to `root`.
pub fn archive(root: &Path, rel: &Path, archive_dir: &Path) -> io::Result<PathBuf> {
    let base = archive_dir.join(rel);
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = base
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut target = base.clone();
    let mut n = 0;
    while fs::symlink_metadata(root.join(&target)).is_ok() {
        n += 1;
        target = base.with_file_name(format!("{stem}-{n}{ext}"));
    }
    rename_within(root, rel, &target)?;
    Ok(target)
}

/// An exclusive lock on an owner-only file, held until it is dropped or its process dies (`flock`
/// on Unix, `LockFileEx` on Windows). One process writes the index; others only probe.
pub struct Lock {
    _file: fs::File,
}

impl Lock {
    /// Waits for the lock.
    pub fn acquire(path: &Path) -> io::Result<Lock> {
        let file = lock_file(path)?;
        file.lock()?;
        Ok(Lock { _file: file })
    }

    /// Takes the lock, or `None` while a writer holds it. A reader's probe holds the lock shared
    /// for an instant, so a refusal while a shared lock can still be taken is a probe, not a
    /// writer, and the lock is tried again.
    pub fn try_acquire(path: &Path) -> io::Result<Option<Lock>> {
        let file = lock_file(path)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(Lock { _file: file })),
                Err(fs::TryLockError::Error(e)) => return Err(e),
                Err(fs::TryLockError::WouldBlock) => {
                    let probe = fs::File::open(path)?;
                    if probe.try_lock_shared().is_err() || std::time::Instant::now() > deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    }

    /// Whether a writer holds the lock, by a shared probe that never blocks one.
    pub fn is_held(path: &Path) -> bool {
        match fs::File::open(path) {
            Ok(file) => matches!(file.try_lock_shared(), Err(fs::TryLockError::WouldBlock)),
            Err(_) => false,
        }
    }
}

fn lock_file(path: &Path) -> io::Result<fs::File> {
    let (dir, name) = match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => (dir, name),
        _ => return Err(io::Error::new(ErrorKind::InvalidInput, "not a file path")),
    };
    ensure_private_file(dir, Path::new(name))?;
    fs::OpenOptions::new().read(true).write(true).open(path)
}
