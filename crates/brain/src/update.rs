//! Signed releases: finding, checking and installing the next one.
//!
//! A release archive is `brain-<version>-<target>.tar.gz` holding one folder, `brain-<version>/`,
//! with the binary and `lib/` beside it. The folder replaces the install's `brain-current` folder
//! whole, so the binary and the runtime library it loads always come from one release. Nothing is
//! replaced until the archive's zipsign signature checks against a key this build was shipped with
//! and, on macOS, the binary and the library inside it are signed by omit's Apple team.

use std::collections::BTreeMap;
use std::env::consts::{ARCH, DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX, OS};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use brain_platform::fs::Lock;
use self_update::http::HeaderMap;
use self_update::http_client::{HttpClient, HttpResponse};
use self_update::{Release, ReleaseAsset, ReleaseSource};

/// The protocol a release speaks; it changes when two brains of different releases can no longer
/// work together, never with an ordinary version bump.
pub const PROTOCOL: u32 = 1;

const RELEASE_KEY: [u8; 32] = *include_bytes!("keys/release.pub");

/// omit's Apple Developer team, whose Developer ID signs every macOS release.
const TEAM_ID: &str = include_str!("keys/team_id").trim_ascii();

/// A release archive is tens of megabytes; five minutes is under 1 Mbit/s for it.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The release listing is a few kilobytes.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// Held beside `brain-current` (never inside it: the swap renames that folder) for a whole
/// install, so the daemon's own update and a `brain update` run by hand never swap at once.
const UPDATE_LOCK: &str = "update.lock";

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

/// Whether this process runs the brain inside `install`. Both paths are canonicalized, so a link
/// into the install (install.sh's `~/.local/bin/brain`) counts; a path that cannot be resolved
/// does not.
pub fn runs_from(install: &Path) -> bool {
    std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .ok()
        .zip(install.canonicalize().ok())
        .is_some_and(|(exe, install)| exe.starts_with(install))
}

/// The binary's file name inside a release folder.
pub fn exe_name() -> String {
    format!("brain{EXE_SUFFIX}")
}

/// The ONNX Runtime library's file name inside a release folder's `lib/`.
pub fn runtime_name() -> String {
    format!("{DLL_PREFIX}onnxruntime{DLL_SUFFIX}")
}

/// What a release must carry before it is installed: a zipsign signature by one of `keys`, and,
/// when `requirement` is set, a code signature on its binary and runtime library meeting that
/// code requirement (`codesign -R`).
pub struct Trust {
    pub keys: Vec<[u8; 32]>,
    pub requirement: Option<String>,
}

/// The trust this build checks releases against: the shipped trust, or a test's key alone.
pub fn release_trust() -> Trust {
    match test_key() {
        Some(key) => Trust {
            keys: vec![key],
            requirement: None,
        },
        None => shipped_trust(),
    }
}

/// The trust this build was shipped with. The committed key placeholder (all zero bytes) ships no
/// key, so a build made before the release key existed can check nothing and installs nothing. On
/// macOS a release is also signed with omit's Developer ID, so holding the release key alone
/// cannot put code on a Mac.
pub fn shipped_trust() -> Trust {
    Trust {
        keys: if RELEASE_KEY == [0; 32] {
            Vec::new()
        } else {
            vec![RELEASE_KEY]
        },
        requirement: brain_platform::codesign::release_requirement(TEAM_ID),
    }
}

/// The key a test signs its releases with, in hex in `BRAIN_TEST_RELEASE_KEY`, so the commands
/// that install can be tested whole; release builds never read it. A test release is checked
/// against that key alone: no test can sign with omit's Developer ID.
fn test_key() -> Option<[u8; 32]> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let hex = std::env::var("BRAIN_TEST_RELEASE_KEY").ok()?;
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect::<Option<_>>()?;
    bytes.try_into().ok()
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
pub fn check(source: impl Source, version: &str) -> anyhow::Result<Option<String>> {
    let nothing = Trust {
        keys: Vec::new(),
        requirement: None,
    };
    let newer = configure(source, &nothing, Path::new("."), version)?.is_update_available()?;
    Ok(newer.map(|r| r.version().to_string()))
}

/// Installs the newest release `source` offers over `current`, the install's `brain-current`
/// folder, once it meets `trust`. Returns the version now installed.
pub fn run(
    source: impl Source,
    trust: &Trust,
    current: &Path,
    version: &str,
) -> anyhow::Result<String> {
    require(&trust.keys)?;
    let install = current
        .parent()
        .context("the install folder has no parent")?;
    let _lock = Lock::acquire(&install.join(UPDATE_LOCK)).context("waiting for another update")?;
    let status = configure(source, trust, current, version)?.update()?;
    Ok(status.version().to_string())
}

/// Whether the binary and the runtime library in the release folder `release`, unpacked and not
/// yet installed, are signed as `requirement` asks.
fn meets(release: &Path, requirement: &str) -> self_update::errors::Result<()> {
    for file in [
        release.join(exe_name()),
        release.join("lib").join(runtime_name()),
    ] {
        brain_platform::codesign::check(&file, requirement).map_err(|why| {
            self_update::errors::Error::verification_rejected(format!(
                "{} is not signed by omit's Apple team ({TEAM_ID}): {why}",
                file.display()
            ))
        })?;
    }
    Ok(())
}

fn configure(
    source: impl Source,
    trust: &Trust,
    current: &Path,
    version: &str,
) -> anyhow::Result<self_update::backends::custom::Update> {
    let mut b = self_update::backends::custom::Update::configure();
    if let Some(client) = source.client() {
        b.http_client(client);
    }
    if let Some(requirement) = trust.requirement.clone() {
        b.verify_binary(move |release| meets(release, &requirement));
    }
    b.timeout(source.timeout());
    Ok(b.source(source)
        .bin_name("brain")
        .target(host_target()?)
        .current_version(version)
        .bundle_path_in_archive("brain-{{ version }}")
        .bundle_install_path(current)
        .verifying_keys(trust.keys.clone())
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()?)
}

/// Where releases come from, and how their assets are fetched.
pub trait Source: ReleaseSource + 'static {
    /// The transport for this source's asset URLs; self_update's own HTTPS client when `None`.
    fn client(&self) -> Option<Arc<dyn HttpClient>> {
        None
    }

    /// The most one download may take, body included.
    fn timeout(&self) -> Duration {
        DOWNLOAD_TIMEOUT
    }
}

/// brain's releases on GitHub.
pub struct GitHub;

impl Source for GitHub {}

/// A folder of release archives, named as a release names them, for tests and for installing
/// without the network. Its archives pass the same signature check as GitHub's.
pub struct LocalDir(pub PathBuf);

const LOCAL: &str = "local:";

impl ReleaseSource for LocalDir {
    fn get_releases(&self) -> self_update::errors::Result<Vec<Release>> {
        let mut by_version: BTreeMap<String, Vec<ReleaseAsset>> = BTreeMap::new();
        for entry in std::fs::read_dir(&self.0)? {
            let path = entry?.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let version = name
                .strip_prefix("brain-")
                .and_then(|n| n.strip_suffix(".tar.gz"))
                .and_then(|n| n.split_once('-'))
                .map(|(v, _target)| v.to_string());
            if let Some(v) = version {
                let url = format!("{LOCAL}{}", path.display());
                by_version
                    .entry(v)
                    .or_default()
                    .push(ReleaseAsset::new(&*name, &url));
            }
        }
        by_version
            .into_iter()
            .map(|(v, assets)| {
                Release::builder()
                    .name(format!("v{v}"))
                    .version(v)
                    .assets(assets)
                    .build()
            })
            .collect()
    }
}

impl Source for LocalDir {
    fn client(&self) -> Option<Arc<dyn HttpClient>> {
        Some(Arc::new(Files))
    }
}

/// Reads `local:` URLs from disk; it fetches nothing else.
struct Files;

struct FileBody(std::fs::File, HeaderMap);

impl HttpResponse for FileBody {
    fn headers(&self) -> &HeaderMap {
        &self.1
    }

    fn body(self: Box<Self>) -> Box<dyn std::io::Read> {
        Box::new(self.0)
    }
}

impl HttpClient for Files {
    fn get(
        &self,
        url: &str,
        _headers: &HeaderMap,
        _timeout: Option<std::time::Duration>,
    ) -> self_update::errors::Result<Box<dyn HttpResponse>> {
        let path = url.strip_prefix(LOCAL).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{url} is not a local release"),
            )
        })?;
        Ok(Box::new(FileBody(
            std::fs::File::open(path)?,
            HeaderMap::new(),
        )))
    }
}

impl ReleaseSource for GitHub {
    fn get_releases(&self) -> self_update::errors::Result<Vec<Release>> {
        self_update::backends::github::ReleaseList::configure()
            .repo_owner("omit-nz")
            .repo_name("brain")
            .timeout(LIST_TIMEOUT)
            .build()?
            .fetch()
            .map(|r| r.into_vec())
    }
}
