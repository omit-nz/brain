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
    let name = format!("brain-{version}-{}.tar.gz", update::host_target().unwrap());
    let unsigned = dir.join(format!("unsigned-{name}"));
    {
        let gz = flate2::write::GzEncoder::new(
            fs::File::create(&unsigned).unwrap(),
            flate2::Compression::default(),
        );
        let mut tar = tar::Builder::new(gz);
        for (path, body) in [
            (
                format!("brain-{version}/{}", update::exe_name()),
                format!("binary {version}"),
            ),
            (
                format!("brain-{version}/lib/{}", update::runtime_name()),
                format!("runtime {version}"),
            ),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            tar.append_data(&mut h, path, body.as_bytes()).unwrap();
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

/// An install of the current version, and the update path pointed at `archive`.
fn install_and_update(
    archive: &Path,
    keys: &[[u8; 32]],
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
        keys,
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
    let keys = update::release_keys();
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
    if !update::release_keys().is_empty() {
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
    let r = update::run(Local { assets }, &[public(&k)], &current, "0.1.0");
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
