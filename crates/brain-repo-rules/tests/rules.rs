use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use yaml_rust2::{Yaml, YamlLoader};

/// SHA-256 of the Apache License 2.0 text with LF line ends, as published in
/// serde 1.0.219's LICENSE-APACHE.
const APACHE_2_SHA256: &str = "62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a";

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// GitHub-hosted runner labels a public repo may use. A self-hosted runner would let a
/// public pull request run code on omit's own machines.
const HOSTED: &[&str] = &["macos-15", "ubuntu-24.04", "windows-2025"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn toml_at(path: &Path) -> toml::Table {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.parse()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn crate_manifests() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(root().join("crates"))
        .unwrap()
        .map(|e| e.unwrap().path().join("Cargo.toml"))
        .filter(|p| p.exists())
        .collect();
    out.sort();
    out
}

fn manifests_with_workspace(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if path.is_dir() {
            if name == ".git" || name == "target" {
                continue;
            }
            manifests_with_workspace(&path, found);
        } else if name == "Cargo.toml" && toml_at(&path).contains_key("workspace") {
            found.push(path);
        }
    }
}

#[test]
fn every_crate_is_apache_2() {
    let root = root();
    let ws = toml_at(&root.join("Cargo.toml"));
    let license = ws["workspace"]["package"]["license"].as_str();
    assert_eq!(license, Some("Apache-2.0"), "workspace licence");

    for manifest in crate_manifests() {
        let pkg = &toml_at(&manifest)["package"];
        let inherits = pkg
            .get("license")
            .and_then(|l| l.get("workspace"))
            .and_then(|w| w.as_bool());
        assert_eq!(
            inherits,
            Some(true),
            "{} must set license.workspace = true",
            manifest.display()
        );
    }

    let text = fs::read_to_string(root.join("LICENSE")).expect("LICENSE");
    let lf = text.replace("\r\n", "\n");
    let digest: String = Sha256::digest(lf.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        digest, APACHE_2_SHA256,
        "LICENSE is not the Apache-2.0 text"
    );
}

/// A crate's manifest takes its version from the workspace, so one release is one version.
fn takes_workspace_version(manifest: &toml::Table) -> Result<(), String> {
    let inherits = manifest
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.get("workspace"))
        .and_then(|w| w.as_bool());
    match inherits {
        Some(true) => Ok(()),
        _ => Err("must set version.workspace = true".into()),
    }
}

#[test]
fn every_crate_takes_the_workspace_version() {
    let ws = toml_at(&root().join("Cargo.toml"));
    assert!(
        ws["workspace"]["package"]["version"].as_str().is_some(),
        "the workspace sets the one version"
    );
    for manifest in crate_manifests() {
        if let Err(e) = takes_workspace_version(&toml_at(&manifest)) {
            panic!("{}: {e}", manifest.display());
        }
    }
    for own in [
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        "[package]\nname = \"x\"\nversion.workspace = false\n",
        "[package]\nname = \"x\"\n",
    ] {
        assert!(
            takes_workspace_version(&own.parse().unwrap()).is_err(),
            "{own} passed"
        );
    }
}

/// Exactly one `[workspace]`, at the root, whose members cover `crates/*` and which excludes
/// nothing.
fn one_workspace_at(root: &Path) -> Result<(), String> {
    let mut found = Vec::new();
    manifests_with_workspace(root, &mut found);
    if found != vec![root.join("Cargo.toml")] {
        return Err(format!(
            "exactly one [workspace], at the root; found {found:?}"
        ));
    }
    let ws = &toml_at(&root.join("Cargo.toml"))["workspace"];
    let members: Vec<&str> = ws["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    if !members.contains(&"crates/*") {
        return Err(format!("members must cover crates/*, got {members:?}"));
    }
    if let Some(ex) = ws.get("exclude") {
        return Err(format!("the workspace excludes {ex}"));
    }
    Ok(())
}

#[test]
fn one_workspace() {
    assert_eq!(one_workspace_at(&root()), Ok(()));
}

#[test]
fn a_second_or_narrowed_workspace_is_caught() {
    let ws = "[workspace]\nmembers = [\"crates/*\"]\n";
    let fixture = |files: &[(&str, &str)]| {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    };
    let (_d, ok) = fixture(&[("Cargo.toml", ws)]);
    assert_eq!(one_workspace_at(&ok), Ok(()));
    let (_d, hidden) = fixture(&[("Cargo.toml", ws), (".hidden/x/Cargo.toml", ws)]);
    assert!(
        one_workspace_at(&hidden).is_err(),
        "a workspace in a hidden folder"
    );
    let narrowed = "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/brain-note\"]\n";
    let (_d, excluded) = fixture(&[("Cargo.toml", narrowed)]);
    assert!(one_workspace_at(&excluded).is_err(), "a workspace exclude");
}

#[test]
fn every_dependency_is_from_crates_io() {
    let root = root();
    let ours: Vec<String> = crate_manifests()
        .iter()
        .map(|m| toml_at(m)["package"]["name"].as_str().unwrap().to_string())
        .collect();
    let lock = toml_at(&root.join("Cargo.lock"));
    for pkg in lock["package"].as_array().unwrap() {
        let name = pkg["name"].as_str().unwrap();
        let source = pkg.get("source").and_then(|s| s.as_str());
        if ours.iter().any(|o| o == name) && source.is_none() {
            continue;
        }
        assert_eq!(source, Some(CRATES_IO), "{name} must come from crates.io");
    }
}

/// Every `runs-on` in one workflow, checked against `HOSTED`. A `${{ matrix.<key> }}` is
/// expanded from that job's `strategy.matrix`; a list or any other expression is refused.
fn hosted_only(workflow: &str) -> Result<(), String> {
    let docs = YamlLoader::load_from_str(workflow).map_err(|e| e.to_string())?;
    let jobs = docs[0]["jobs"].as_hash().ok_or("no jobs")?;
    for (name, job) in jobs {
        let name = name.as_str().unwrap_or("?");
        let label = match &job["runs-on"] {
            Yaml::String(s) => s.clone(),
            other => return Err(format!("{name}: runs-on must be one label, got {other:?}")),
        };
        let labels = match label
            .strip_prefix("${{ matrix.")
            .and_then(|k| k.strip_suffix(" }}"))
        {
            Some(key) => job["strategy"]["matrix"][key]
                .as_vec()
                .ok_or(format!("{name}: matrix.{key} is not a list"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or(format!("{name}: matrix.{key} holds a non-string"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            None if label.contains("${{") => {
                return Err(format!("{name}: runs-on expression {label}"));
            }
            None => vec![label],
        };
        for l in labels {
            if !HOSTED.contains(&l.as_str()) {
                return Err(format!("{name}: {l} is not a GitHub-hosted runner"));
            }
        }
    }
    Ok(())
}

#[test]
fn ci_uses_hosted_runners_only() {
    let dir = root().join(".github/workflows");
    let mut seen = 0;
    for entry in fs::read_dir(&dir).expect(".github/workflows") {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "yml" || e == "yaml") {
            hosted_only(&fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            seen += 1;
        }
    }
    assert!(seen > 0, "no workflows found");
}

#[test]
fn self_hosted_labels_are_refused() {
    let list = "jobs:\n  a:\n    runs-on: [self-hosted, macos-pr]\n";
    let release = "jobs:\n  a:\n    runs-on: macos-release\n";
    let matrix = "jobs:\n  a:\n    strategy:\n      matrix:\n        os: [ubuntu-24.04, macos-pr]\n    runs-on: ${{ matrix.os }}\n";
    let ok = "jobs:\n  a:\n    strategy:\n      matrix:\n        os: [ubuntu-24.04, windows-2025]\n    runs-on: ${{ matrix.os }}\n";
    assert!(hosted_only(list).is_err());
    assert!(hosted_only(release).is_err());
    assert!(hosted_only(matrix).is_err());
    assert_eq!(hosted_only(ok), Ok(()));
}

/// Every `.rs` file under `dir`, as (path, text).
fn rust_files(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).unwrap();
            out.push((path, text));
        }
    }
}

/// The files that name `CLAUDE.md`, exactly as written: brain reaches an AI tool through its MCP
/// door and its own `claude mcp` command, never by writing the person's instructions file. Naming
/// the file is the proxy for writing it; a path built from pieces would slip past, and review
/// catches that.
fn naming_claude_md(files: &[(PathBuf, String)]) -> Vec<String> {
    files
        .iter()
        .filter(|(_, text)| text.contains("CLAUDE.md"))
        .map(|(path, _)| path.display().to_string())
        .collect()
}

#[test]
fn nothing_writes_claude_md() {
    let mut files = Vec::new();
    for krate in fs::read_dir(root().join("crates")).unwrap() {
        let src = krate.unwrap().path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    assert!(!files.is_empty());
    assert_eq!(naming_claude_md(&files), Vec::<String>::new());
}

#[test]
fn a_file_naming_claude_md_is_caught() {
    let files = [
        (
            PathBuf::from("crates/brain/src/connect.rs"),
            "fs::write(home.join(\"CLAUDE.md\"), rules)".to_string(),
        ),
        (
            PathBuf::from("crates/brain/src/ok.rs"),
            "fn ok() {}".to_string(),
        ),
    ];
    assert_eq!(naming_claude_md(&files), ["crates/brain/src/connect.rs"]);
}
