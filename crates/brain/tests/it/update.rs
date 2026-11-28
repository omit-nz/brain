use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use brain::update::{self, PROTOCOL};
use self_update::{Release, ReleaseAsset, ReleaseSource};
use zipsign_api::SigningKey;

const NEW: &str = "9.9.9";

pub(crate) fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

pub(crate) fn public(k: &SigningKey) -> [u8; 32] {
    k.verifying_key().to_bytes()
}

/// `brain-<version>-<target>.tar.gz` holding `brain-<version>/brain` and its runtime library,
/// signed by every key in `signers` (none: unsigned), with the archive's file name as the signing
/// context.
pub(crate) fn archive(dir: &Path, version: &str, signers: &[SigningKey]) -> PathBuf {
    archive_holding(dir, version, signers, &format!("binary {version}"))
}

/// `archive`, with `exe` as the release's brain.
pub(crate) fn archive_holding(
    dir: &Path,
    version: &str,
    signers: &[SigningKey],
    exe: &str,
) -> PathBuf {
    archive_of(
        dir,
        version,
        signers,
        exe.as_bytes(),
        format!("runtime {version}").as_bytes(),
    )
}

/// `archive`, with `exe` as the release's brain and `runtime` as its runtime library.
fn archive_of(
    dir: &Path,
    version: &str,
    signers: &[SigningKey],
    exe: &[u8],
    runtime: &[u8],
) -> PathBuf {
    archive_with(
        dir,
        version,
        signers,
        &[
            (update::exe_name(), Entry::File(exe)),
            (
                format!("lib/{}", update::runtime_name()),
                Entry::File(runtime),
            ),
        ],
    )
}

/// One entry of a release folder.
enum Entry<'a> {
    File(&'a [u8]),
    #[cfg(target_os = "macos")]
    Link(&'a Path),
}

/// `archive`, holding `entries` (paths inside the release folder).
fn archive_with(
    dir: &Path,
    version: &str,
    signers: &[SigningKey],
    entries: &[(String, Entry)],
) -> PathBuf {
    let name = format!("brain-{version}-{}.tar.gz", update::host_target().unwrap());
    let unsigned = dir.join(format!("unsigned-{name}"));
    {
        let gz = flate2::write::GzEncoder::new(
            fs::File::create(&unsigned).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(gz);
        for (path, entry) in entries {
            let path = format!("brain-{version}/{path}");
            let mut h = tar::Header::new_gnu();
            h.set_mode(0o755);
            match entry {
                Entry::File(body) => {
                    h.set_size(body.len() as u64);
                    h.set_cksum();
                    tar.append_data(&mut h, path, *body).unwrap();
                }
                #[cfg(target_os = "macos")]
                Entry::Link(target) => {
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_size(0);
                    tar.append_link(&mut h, path, target).unwrap();
                }
            }
        }
        tar.into_inner().unwrap().finish().unwrap();
    }
    let out = dir.join(&name);
    match signers {
        [] => {
            fs::copy(&unsigned, &out).unwrap();
        }
        keys => {
            let mut input = fs::File::open(&unsigned).unwrap();
            let mut output = fs::File::create(&out).unwrap();
            zipsign_api::sign::copy_and_sign_tar(
                &mut input,
                &mut output,
                keys,
                Some(name.as_bytes()),
            )
            .unwrap();
        }
    }
    out
}

/// Serves `file` to every request on a local port, for `hits` requests.
fn serve(file: PathBuf, hits: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(hits) {
            let mut s = stream.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let body = fs::read(&file).unwrap();
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    format!("http://{addr}")
}

/// A release listing of `assets`, (name, url) pairs, in order.
struct Local {
    assets: Vec<(String, String)>,
}

impl ReleaseSource for Local {
    fn get_releases(&self) -> self_update::errors::Result<Vec<Release>> {
        Ok(vec![
            Release::builder()
                .version(NEW)
                .name(format!("v{NEW}"))
                .assets(self.assets.iter().map(|(n, u)| ReleaseAsset::new(n, u)))
                .build()?,
        ])
    }
}

impl update::Source for Local {}

/// Trusting releases signed by any of `keys`, with no code signature asked for.
fn trusting(keys: &[[u8; 32]]) -> update::Trust {
    update::Trust {
        keys: keys.to_vec(),
        code: None,
    }
}

/// An install of the current version, and the update path pointed at `archive`.
fn install_and_update(
    archive: &Path,
    keys: &[[u8; 32]],
) -> (tempfile::TempDir, anyhow::Result<String>) {
    install_and_update_with(archive, &trusting(keys))
}

/// `install_and_update`, checking the release against `trust`.
fn install_and_update_with(
    archive: &Path,
    trust: &update::Trust,
) -> (tempfile::TempDir, anyhow::Result<String>) {
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("brain-current");
    fs::create_dir_all(current.join("lib")).unwrap();
    fs::write(current.join(update::exe_name()), "binary old").unwrap();
    fs::write(
        current.join("lib").join(update::runtime_name()),
        "runtime old",
    )
    .unwrap();
    let name = archive.file_name().unwrap().to_string_lossy().to_string();
    let url = format!("{}/{name}", serve(archive.to_path_buf(), 4));
    let r = update::run(
        Local {
            assets: vec![(name, url)],
        },
        trust,
        &current,
        "0.1.0",
    );
    (root, r)
}

fn installed(root: &Path) -> (String, String) {
    let current = root.join("brain-current");
    (
        fs::read_to_string(current.join(update::exe_name())).unwrap(),
        fs::read_to_string(current.join("lib").join(update::runtime_name())).unwrap(),
    )
}

#[test]
fn update_refuses_an_unsigned_archive() {
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[]);
    let (root, r) = install_and_update(&a, &[public(&key(1))]);
    assert!(r.is_err(), "an unsigned archive was installed");
    assert_eq!(
        installed(root.path()),
        ("binary old".into(), "runtime old".into())
    );
}

#[test]
fn update_refuses_an_archive_signed_by_another_key() {
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[key(2)]);
    let (root, r) = install_and_update(&a, &[public(&key(1))]);
    assert!(r.is_err(), "an archive signed by a stranger was installed");
    assert_eq!(
        installed(root.path()),
        ("binary old".into(), "runtime old".into())
    );
}

#[test]
fn update_replaces_binary_and_runtime_together() {
    let dir = tempfile::tempdir().unwrap();
    let k = key(1);
    let a = archive(dir.path(), NEW, std::slice::from_ref(&k));
    let (root, r) = install_and_update(&a, &[public(&k)]);
    assert_eq!(r.unwrap(), NEW);
    assert_eq!(
        installed(root.path()),
        (format!("binary {NEW}"), format!("runtime {NEW}"))
    );
}

#[test]
fn update_without_a_key_is_refused_before_any_download() {
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[key(1)]);
    let (root, r) = install_and_update(&a, &[]);
    let err = format!("{:#}", r.unwrap_err());
    assert!(err.contains("no release key"), "{err}");
    assert_eq!(
        installed(root.path()),
        ("binary old".into(), "runtime old".into())
    );
}

#[test]
fn check_names_a_newer_release_and_installs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[key(1)]);
    let name = a.file_name().unwrap().to_string_lossy().to_string();
    let local = |name: &str| Local {
        assets: vec![(
            name.to_string(),
            "http://127.0.0.1:9/never-fetched".to_string(),
        )],
    };
    assert_eq!(
        update::check(local(&name), "0.1.0").unwrap(),
        Some(NEW.to_string())
    );
    assert_eq!(update::check(local(&name), NEW).unwrap(), None);
}

#[test]
fn install_dir_names_the_folder_update_replaces() {
    let home = tempfile::tempdir().unwrap();
    let out = assert_cmd::Command::cargo_bin("brain")
        .unwrap()
        .env("BRAIN_HOME", home.path())
        .arg("install-dir")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        home.path()
            .join("data")
            .join("bin")
            .join("brain-current")
            .to_string_lossy()
    );
}

#[test]
fn update_from_a_build_with_no_install_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let out = assert_cmd::Command::cargo_bin("brain")
        .unwrap()
        .env("BRAIN_HOME", home.path())
        .arg("update")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("brain updates only an installed brain, so install brain"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn release_verify_refuses_a_changed_byte() {
    let dir = tempfile::tempdir().unwrap();
    let k = key(1);
    let a = archive(dir.path(), NEW, std::slice::from_ref(&k));
    update::verify(&a, &[public(&k)]).unwrap();
    let mut bytes = fs::read(&a).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x01;
    fs::write(&a, bytes).unwrap();
    assert!(update::verify(&a, &[public(&k)]).is_err());
}

#[test]
fn the_shipped_key_set_is_the_committed_key() {
    let committed =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/keys/release.pub")).unwrap();
    assert_eq!(committed.len(), 32);
    let keys = update::shipped_trust().keys;
    if committed.iter().all(|b| *b == 0) {
        assert!(keys.is_empty(), "the all-zero placeholder must ship no key");
    } else {
        assert_eq!(
            keys,
            vec![<[u8; 32]>::try_from(committed.as_slice()).unwrap()]
        );
    }
}

#[test]
fn version_prints_the_protocol() {
    let out = assert_cmd::Command::cargo_bin("brain")
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("brain {} (protocol {PROTOCOL})", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn release_verify_without_a_key_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[key(1)]);
    if !update::shipped_trust().keys.is_empty() {
        return;
    }
    let out = assert_cmd::Command::cargo_bin("brain")
        .unwrap()
        .env("BRAIN_HOME", dir.path())
        .args(["release-verify", a.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no release key"));
}

#[test]
fn verify_with_no_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[key(1)]);
    let err = update::verify(&a, &[]).unwrap_err().to_string();
    assert!(err.contains("no release key"), "{err}");
}

#[test]
fn an_archive_signed_by_both_keys_updates_an_install_trusting_either() {
    for trusted in [key(1), key(2)] {
        let dir = tempfile::tempdir().unwrap();
        let a = archive(dir.path(), NEW, &[key(1), key(2)]);
        let (root, r) = install_and_update(&a, &[public(&trusted)]);
        assert_eq!(r.unwrap(), NEW);
        assert_eq!(
            installed(root.path()),
            (format!("binary {NEW}"), format!("runtime {NEW}"))
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[key(1), key(2)]);
    let (root, r) = install_and_update(&a, &[public(&key(3))]);
    assert!(r.is_err(), "a third key's install took a two-key archive");
    assert_eq!(
        installed(root.path()),
        ("binary old".into(), "runtime old".into())
    );
}

#[test]
fn update_picks_the_host_archive_among_every_asset() {
    let dir = tempfile::tempdir().unwrap();
    let k = key(1);
    let a = archive(dir.path(), NEW, std::slice::from_ref(&k));
    let host = update::host_target().unwrap();
    // A decoy is never served: fetching one fails the update.
    let closed = "http://127.0.0.1:9/decoy".to_string();
    let mut assets: Vec<(String, String)> = [
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
    ]
    .into_iter()
    .filter(|t| *t != host)
    .map(|t| (format!("brain-{NEW}-{t}.tar.gz"), closed.clone()))
    .collect();
    assets.push(("SHA256SUMS".into(), closed.clone()));
    assets.push(("release.json".into(), closed.clone()));
    let name = a.file_name().unwrap().to_string_lossy().to_string();
    assets.push((name.clone(), format!("{}/{name}", serve(a.clone(), 4))));

    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("brain-current");
    fs::create_dir_all(current.join("lib")).unwrap();
    fs::write(current.join(update::exe_name()), "binary old").unwrap();
    fs::write(
        current.join("lib").join(update::runtime_name()),
        "runtime old",
    )
    .unwrap();
    let r = update::run(
        Local { assets },
        &trusting(&[public(&k)]),
        &current,
        "0.1.0",
    );
    assert_eq!(r.unwrap(), NEW);
    assert_eq!(
        installed(root.path()),
        (format!("binary {NEW}"), format!("runtime {NEW}"))
    );
}

#[test]
fn a_signed_archive_under_another_name_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let k = key(1);
    let a = archive(dir.path(), NEW, std::slice::from_ref(&k));
    let renamed = dir.path().join(format!(
        "brain-9.9.8-{}.tar.gz",
        update::host_target().unwrap()
    ));
    fs::copy(&a, &renamed).unwrap();
    update::verify(&a, &[public(&k)]).unwrap();
    assert!(update::verify(&renamed, &[public(&k)]).is_err());
}

/// An install laid out as scripts/install.sh lays it out: the release folder in `bin/`, and a
/// link to its binary in `local/` standing in for ~/.local/bin.
#[cfg(unix)]
#[test]
fn an_install_run_through_its_link_finds_its_runtime() {
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("bin").join("brain-current");
    fs::create_dir_all(current.join("lib")).unwrap();
    fs::copy(
        assert_cmd::cargo::cargo_bin("brain"),
        current.join(update::exe_name()),
    )
    .unwrap();
    let runtime = current.join("lib").join(update::runtime_name());
    fs::write(&runtime, "runtime").unwrap();
    fs::create_dir_all(root.path().join("local")).unwrap();
    let link = root.path().join("local").join("brain");
    std::os::unix::fs::symlink(current.join(update::exe_name()), &link).unwrap();

    let home = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = std::process::Command::new(&link)
            .env("BRAIN_HOME", home.path())
            .env_remove("ORT_DYLIB_PATH")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    run(&[
        "init",
        "--me",
        "r",
        "--vault",
        vault.path().to_str().unwrap(),
    ]);
    let status: serde_json::Value = serde_json::from_slice(&run(&["status", "--json"])).unwrap();
    let found = status["runtime"]
        .as_str()
        .map(|p| fs::canonicalize(p).unwrap());
    assert_eq!(found, Some(fs::canonicalize(&runtime).unwrap()), "{status}");
}

/// A release listing whose one asset sits on a host that accepts and never answers.
struct Silent {
    url: String,
}

impl ReleaseSource for Silent {
    fn get_releases(&self) -> self_update::errors::Result<Vec<Release>> {
        let name = format!("brain-{NEW}-{}.tar.gz", update::host_target().unwrap());
        Ok(vec![
            Release::builder()
                .version(NEW)
                .name(format!("v{NEW}"))
                .assets([ReleaseAsset::new(&name, &self.url)])
                .build()?,
        ])
    }
}

impl update::Source for Silent {
    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }
}

#[test]
fn a_silent_release_host_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/brain.tar.gz", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            held.push(stream);
        }
    });
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("brain-current");
    fs::create_dir_all(&current).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = update::run(
            Silent { url },
            &trusting(&[public(&key(1))]),
            &current,
            "0.1.0",
        );
        let _ = tx.send(r.is_err());
    });
    let failed = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the download never timed out");
    assert!(failed);
}

#[test]
fn an_update_waits_for_one_already_running() {
    let dir = tempfile::tempdir().unwrap();
    let a = archive(dir.path(), NEW, &[key(1)]);
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("brain-current");
    fs::create_dir_all(current.join("lib")).unwrap();
    fs::write(current.join(update::exe_name()), "binary old").unwrap();
    let held = brain_platform::fs::Lock::acquire(&root.path().join("update.lock")).unwrap();
    let name = a.file_name().unwrap().to_string_lossy().to_string();
    let url = format!("{}/{name}", serve(a.clone(), 4));
    let (tx, rx) = std::sync::mpsc::channel();
    let target = current.clone();
    std::thread::spawn(move || {
        let r = update::run(
            Local {
                assets: vec![(name, url)],
            },
            &trusting(&[public(&key(1))]),
            &target,
            "0.1.0",
        );
        let _ = tx.send(r.map_err(|e| e.to_string()));
    });
    assert!(
        rx.recv_timeout(std::time::Duration::from_secs(2)).is_err(),
        "an update ran while another held the install"
    );
    drop(held);
    let r = rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap();
    assert_eq!(r.unwrap(), NEW);
    assert_eq!(installed(root.path()).0, format!("binary {NEW}"));
}

/// An install whose update is checked against `code`, offered a release of `exe` and `runtime`
/// signed by the trusted key.
#[cfg(target_os = "macos")]
fn update_to(exe: &[u8], runtime: &[u8], code: update::Code) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let k = key(1);
    let a = archive_of(dir.path(), NEW, std::slice::from_ref(&k), exe, runtime);
    let (root, r) = install_and_update_with(
        &a,
        &update::Trust {
            keys: vec![public(&k)],
            code: Some(code),
        },
    );
    (
        root,
        r.map_err(|e| format!("{e:#}")).err().unwrap_or_default(),
    )
}

/// An Apple-signed binary every Mac has: signed, but not by omit's team.
#[cfg(target_os = "macos")]
fn apple_signed() -> Vec<u8> {
    fs::read("/bin/ls").unwrap()
}

#[cfg(target_os = "macos")]
fn shipped_code() -> update::Code {
    update::shipped_trust().code.unwrap()
}

/// `requirement` asked of both files.
#[cfg(target_os = "macos")]
fn both(requirement: &str) -> update::Code {
    update::Code {
        brain: requirement.to_string(),
        runtime: requirement.to_string(),
    }
}

/// What a Developer ID Application certificate of omit's team carries, in Apple's own form: Apple's
/// Developer ID intermediate, the Developer ID leaf marker, and the team.
#[cfg(target_os = "macos")]
const DEVELOPER_ID: &str = r#"anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "5BCNYRT23U""#;

#[cfg(target_os = "macos")]
#[test]
fn the_shipped_trust_on_macos_asks_for_brains_developer_id() {
    let team =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/keys/team_id")).unwrap();
    assert_eq!(team, "5BCNYRT23U");
    let code = shipped_code();
    assert_eq!(
        code.brain,
        format!(r#"identifier "nz.omit.brain" and {DEVELOPER_ID}"#)
    );
    assert_eq!(code.runtime, DEVELOPER_ID);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_shipped_trust_elsewhere_asks_for_no_code_signature() {
    assert!(update::shipped_trust().code.is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn an_unsigned_update_is_refused_on_macos() {
    let (root, err) = update_to(b"binary 9.9.9", b"runtime 9.9.9", shipped_code());
    assert!(
        err.contains("does not carry brain's Developer ID signature"),
        "{err:?}"
    );
    assert_eq!(
        installed(root.path()),
        ("binary old".into(), "runtime old".into())
    );
}

#[cfg(target_os = "macos")]
#[test]
fn an_update_signed_by_another_team_is_refused() {
    let (root, err) = update_to(&apple_signed(), &apple_signed(), shipped_code());
    assert!(
        err.contains("does not carry brain's Developer ID signature"),
        "{err:?}"
    );
    assert_eq!(
        installed(root.path()),
        ("binary old".into(), "runtime old".into())
    );
}

/// `/bin/ls` meets `anchor apple`, which stands in for omit's team: no test holds its Developer ID.
#[cfg(target_os = "macos")]
#[test]
fn an_update_whose_brain_and_runtime_meet_the_requirement_is_installed() {
    let (root, err) = update_to(&apple_signed(), &apple_signed(), both("anchor apple"));
    assert_eq!(err, "");
    let current = root.path().join("brain-current");
    assert_eq!(
        fs::read(current.join(update::exe_name())).unwrap(),
        apple_signed()
    );
    assert_eq!(
        fs::read(current.join("lib").join(update::runtime_name())).unwrap(),
        apple_signed()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn an_update_with_one_unsigned_file_is_refused() {
    for (exe, runtime) in [
        (apple_signed(), b"runtime 9.9.9".to_vec()),
        (b"binary 9.9.9".to_vec(), apple_signed()),
    ] {
        let (root, err) = update_to(&exe, &runtime, both("anchor apple"));
        assert!(
            err.contains("does not carry brain's Developer ID signature"),
            "{err:?}"
        );
        assert_eq!(
            installed(root.path()),
            ("binary old".into(), "runtime old".into())
        );
    }
}

/// A release whose brain, runtime library or `lib` folder is a link is refused, though what the
/// link points at meets the requirement: the install would hold the link, not the signed file.
#[cfg(target_os = "macos")]
#[test]
fn a_release_holding_a_link_is_refused() {
    let signed = tempfile::tempdir().unwrap();
    let file = signed.path().join(update::runtime_name());
    fs::write(&file, apple_signed()).unwrap();
    let lib = format!("lib/{}", update::runtime_name());
    let ls = apple_signed();
    let cases: [Vec<(String, Entry)>; 3] = [
        vec![
            (update::exe_name(), Entry::Link(&file)),
            (lib.clone(), Entry::File(&ls)),
        ],
        vec![
            (update::exe_name(), Entry::File(&ls)),
            (lib.clone(), Entry::Link(&file)),
        ],
        vec![
            (update::exe_name(), Entry::File(&ls)),
            ("lib".to_string(), Entry::Link(signed.path())),
        ],
    ];
    for entries in cases {
        let dir = tempfile::tempdir().unwrap();
        let k = key(1);
        let a = archive_with(dir.path(), NEW, std::slice::from_ref(&k), &entries);
        let (root, r) = install_and_update_with(
            &a,
            &update::Trust {
                keys: vec![public(&k)],
                code: Some(both("anchor apple")),
            },
        );
        let err = format!(
            "{:#}",
            r.expect_err("a release holding a link was installed")
        );
        assert!(err.contains("not a plain file"), "{err}");
        assert_eq!(
            installed(root.path()),
            ("binary old".into(), "runtime old".into())
        );
    }
}

/// brain's own requirement is asked of brain alone: a signed binary with another identifier (here
/// Apple's `ls`, as another of the team's tools would be) is refused as brain, and the runtime
/// library is asked only its own.
#[cfg(target_os = "macos")]
#[test]
fn a_brain_with_another_identifier_is_refused() {
    // Another Apple binary, with its own identifier, as the runtime library.
    let other = fs::read("/bin/cat").unwrap();
    let code = |brain: &str| update::Code {
        brain: brain.to_string(),
        runtime: "anchor apple".to_string(),
    };
    let (root, err) = update_to(
        &apple_signed(),
        &other,
        code(r#"identifier "nz.omit.brain" and anchor apple"#),
    );
    assert!(
        err.contains("does not carry brain's Developer ID signature"),
        "{err:?}"
    );
    assert_eq!(
        installed(root.path()),
        ("binary old".into(), "runtime old".into())
    );
    let (_, err) = update_to(
        &apple_signed(),
        &other,
        code(r#"identifier "com.apple.ls" and anchor apple"#),
    );
    assert_eq!(err, "");
}

/// A download can take minutes on a slow line, and a blank screen reads as a hang.
#[test]
fn brain_update_says_what_it_is_waiting_for() {
    let h = super::Home::new();
    h.init();
    let current = PathBuf::from(h.run(&["install-dir"]).trim_end());
    fs::create_dir_all(current.join("lib")).unwrap();
    fs::write(current.join(update::exe_name()), "binary old").unwrap();
    fs::write(
        current.join("lib").join(update::runtime_name()),
        "runtime old",
    )
    .unwrap();
    let releases = tempfile::tempdir().unwrap();
    let hex: String = public(&key(7)).iter().map(|b| format!("{b:02x}")).collect();
    let update = || {
        let out = h
            .brain()
            .env("BRAIN_TEST_RELEASE_KEY", &hex)
            // The real one would restart the daemon of whoever runs the tests.
            .env("BRAIN_LAUNCHCTL", "no-launchctl")
            .args(["update", "--from-dir"])
            .arg(releases.path())
            .output()
            .unwrap();
        let said = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        assert!(out.status.success(), "{}", said(&out.stderr));
        (said(&out.stdout), said(&out.stderr))
    };
    let (out, err) = update();
    assert!(err.starts_with("Looking for a newer release...\n"), "{err}");
    assert!(!err.contains("Downloading"), "{err}");
    assert!(out.ends_with("is the newest release.\n"), "{out}");
    archive(releases.path(), "99.0.0", &[key(7)]);
    let (out, err) = update();
    assert!(
        err.contains("Looking for a newer release...\nDownloading brain 99.0.0 and checking it..."),
        "{err}"
    );
    assert!(out.starts_with("brain 99.0.0 installed over "), "{out}");
}
