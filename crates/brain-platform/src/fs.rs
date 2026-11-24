//! Every file write and move brain makes. Nothing here deletes: taking a file away is a move into
//! the archive, so nothing is lost without its owner saying so.

use std::fs;
use std::io::{self, ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

pub enum Privacy {
    /// Owner-only (`0600` on Unix): stores, ledgers, keys.
    Private,
    /// Whatever the process umask allows: notes the person opens in their own editor.
    Normal,
}

pub fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| is_link_meta(&m))
}

#[cfg(windows)]
fn is_link_meta(m: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    m.file_type().is_symlink() || m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_link_meta(m: &fs::Metadata) -> bool {
    m.file_type().is_symlink()
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

pub fn write_atomic(root: &Path, rel: &Path, bytes: &[u8], privacy: Privacy) -> io::Result<()> {
    let path = checked(root, rel)?;
    let parent = parent_dirs(&path)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".brain-");
    if let Some(p) = permissions(privacy) {
        builder.permissions(p);
    }
    let mut tmp = builder.tempfile_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(&path).map_err(|e| e.error)?;
    Ok(())
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

/// Windows files inherit the ACL of the person's profile folder, which is already owner-only.
#[cfg(not(unix))]
fn permissions(_: Privacy) -> Option<fs::Permissions> {
    None
}

pub fn create_dir_private(path: &Path) -> io::Result<()> {
    if is_link(path) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("{} is a link", path.display()),
        ));
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn rename_within(root: &Path, from: &Path, to: &Path) -> io::Result<()> {
    let src = checked(root, from)?;
    let dst = checked(root, to)?;
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
