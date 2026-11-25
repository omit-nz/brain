//! Signed releases: finding, checking and installing the next one.
//!
//! A release archive is `brain-<version>-<target>.tar.gz` holding one folder, `brain-<version>/`,
//! with the binary and `lib/` beside it. The folder replaces the install's `brain-current` folder
//! whole, so the binary and the runtime library it loads always come from one release. Nothing is
//! replaced until the archive's zipsign signature checks against a key this build was shipped with.

use std::collections::BTreeMap;
use std::env::consts::{ARCH, DLL_PREFIX, DLL_SUFFIX, EXE_SUFFIX, OS};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use self_update::http::HeaderMap;
use self_update::http_client::{HttpClient, HttpResponse};
use self_update::{Release, ReleaseAsset, ReleaseSource};

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

/// The keys a release must be signed by. The committed placeholder (all zero bytes) ships none,
/// so a build made before the release key existed can check nothing and installs nothing.
pub fn release_keys() -> Vec<[u8; 32]> {
    if let Some(key) = test_key() {
        return vec![key];
    }
    if RELEASE_KEY == [0; 32] {
        Vec::new()
    } else {
        vec![RELEASE_KEY]
    }
}

/// The key a test signs its releases with, in hex in `BRAIN_TEST_RELEASE_KEY`, so the commands
/// that install can be tested whole; release builds never read it.
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
    let newer = configure(source, &[], Path::new("."), version)?.is_update_available()?;
    Ok(newer.map(|r| r.version().to_string()))
}

/// Installs the newest release `source` offers over `current`, the install's `brain-current`
/// folder, once its signature checks against `keys`. Returns the version now installed.
pub fn run(
    source: impl Source,
    keys: &[[u8; 32]],
    current: &Path,
    version: &str,
) -> anyhow::Result<String> {
    require(keys)?;
    let status = configure(source, keys, current, version)?.update()?;
    Ok(status.version().to_string())
}

fn configure(
    source: impl Source,
    keys: &[[u8; 32]],
    current: &Path,
    version: &str,
) -> anyhow::Result<self_update::backends::custom::Update> {
    let mut b = self_update::backends::custom::Update::configure();
    if let Some(client) = source.client() {
        b.http_client(client);
    }
    Ok(b.source(source)
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

/// Where releases come from, and how their assets are fetched.
pub trait Source: ReleaseSource + 'static {
    /// The transport for this source's asset URLs; self_update's own HTTPS client when `None`.
    fn client(&self) -> Option<Arc<dyn HttpClient>> {
        None
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
            .build()?
            .fetch()
            .map(|r| r.into_vec())
    }
}
