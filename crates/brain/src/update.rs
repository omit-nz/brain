//! Signed releases: finding, checking and installing the next one.
//!
//! A release archive is `brain-<version>-<target>.tar.gz` holding one folder, `brain-<version>/`,
//! with the binary and `lib/` beside it. The folder replaces the install's `brain-current` folder
//! whole, so the binary and the runtime library it loads always come from one release. Nothing is
//! replaced until the archive's zipsign signature checks against a key this build was shipped with.

use std::env::consts::{ARCH, DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX, OS};
use std::path::{Path, PathBuf};

use anyhow::Context;
use self_update::{Release, ReleaseSource};

/// The protocol a release speaks; it changes when two brains of different releases can no longer
/// work together, never with an ordinary version bump.
pub const PROTOCOL: u32 = 1;

const RELEASE_KEY: [u8; 32] = *include_bytes!("keys/release.pub");

/// The target triple this machine's releases are built for.
pub fn host_target() -> anyhow::Result<&'static str> {
    match (ARCH, OS) {
        ("aarch64", "macos") => Ok("aarch64-apple-darwin"),
        ("x86_64", "linux") => Ok("x86_64-unknown-linux-gnu"),
        ("x86_64", "windows") => Ok("x86_64-pc-windows-msvc"),
        _ => anyhow::bail!("brain publishes no release for {ARCH} {OS}"),
    }
}

/// The install: one release folder, replaced whole by each update.
pub fn install_dir(bin: &Path) -> PathBuf {
    bin.join("brain-current")
}

/// The binary's file name inside a release folder.
pub fn exe_name() -> String {
    format!("brain{EXE_SUFFIX}")
}

/// The ONNX Runtime library's file name inside a release folder's `lib/`.
pub fn runtime_name() -> String {
    format!("{DLL_PREFIX}onnxruntime{DLL_SUFFIX}")
}

/// The keys a release must be signed by. The committed placeholder (all zero bytes) ships none,
/// so a build made before the release key existed can check nothing and installs nothing.
pub fn release_keys() -> Vec<[u8; 32]> {
    if RELEASE_KEY == [0; 32] {
        Vec::new()
    } else {
        vec![RELEASE_KEY]
    }
}

/// self_update skips the signature check when it is given no key, so an empty key list stops
/// here instead.
fn require(keys: &[[u8; 32]]) -> anyhow::Result<()> {
    anyhow::ensure!(
        !keys.is_empty(),
        "this build has no release key, so it cannot check a release"
    );
    Ok(())
}

/// Checks `archive`'s signature, with its file name as the signing context.
pub fn verify(archive: &Path, keys: &[[u8; 32]]) -> anyhow::Result<()> {
    require(keys)?;
    self_update::verify_signature(archive, keys)
        .with_context(|| format!("{} is not signed by a brain release key", archive.display()))
}

/// The newest release `source` offers that is newer than `version`, if any.
pub fn check(
    source: impl ReleaseSource + 'static,
    version: &str,
) -> anyhow::Result<Option<String>> {
    let newer = configure(source, &[], Path::new("."), version)?.is_update_available()?;
    Ok(newer.map(|r| r.version().to_string()))
}

/// Installs the newest release `source` offers over `current`, the install's `brain-current`
/// folder, once its signature checks against `keys`. Returns the version now installed.
pub fn run(
    source: impl ReleaseSource + 'static,
    keys: &[[u8; 32]],
    current: &Path,
    version: &str,
) -> anyhow::Result<String> {
    require(keys)?;
    let status = configure(source, keys, current, version)?.update()?;
    Ok(status.version().to_string())
}

fn configure(
    source: impl ReleaseSource + 'static,
    keys: &[[u8; 32]],
    current: &Path,
    version: &str,
) -> anyhow::Result<self_update::backends::custom::Update> {
    Ok(self_update::backends::custom::Update::configure()
        .source(source)
        .bin_name("brain")
        .target(host_target()?)
        .current_version(version)
        .bundle_path_in_archive("brain-{{ version }}")
        .bundle_install_path(current)
        .verifying_keys(keys.to_vec())
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()?)
}

/// brain's releases on GitHub.
pub struct GitHub;

impl ReleaseSource for GitHub {
    fn get_releases(&self) -> self_update::errors::Result<Vec<Release>> {
        self_update::backends::github::ReleaseList::configure()
            .repo_owner("omit-nz")
            .repo_name("brain")
            .build()?
            .fetch()
            .map(|r| r.into_vec())
    }
}
