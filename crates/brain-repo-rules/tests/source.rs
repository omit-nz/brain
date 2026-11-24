//! Scans of the repo's own source: platform code, file writes and the clock each live in one place,
//! and crates depend only downward.

use std::fs;
use std::path::{Path, PathBuf};

/// The crates in layer order; each may depend only on crates before it.
const LAYERS: &[&str] = &["brain-platform", "brain-note", "brain-index", "brain"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every `.rs` file under a crate's `src/`, which is production code; `tests/` is exempt.
fn production_sources() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root().join("crates")).unwrap() {
        let dir = entry.unwrap().path();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let mut files = Vec::new();
        rs_files(&dir.join("src"), &mut files);
        out.extend(files.into_iter().map(|f| (name.clone(), f)));
    }
    out
}

/// The predicate text inside each `cfg(` or `cfg!(` occurrence, with balanced parentheses.
fn cfg_predicates(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(at) = text[i..].find("cfg") {
        let start = i + at + 3;
        let mut j = start;
        if bytes.get(j) == Some(&b'!') {
            j += 1;
        }
        if bytes.get(j) != Some(&b'(') {
            i = start;
            continue;
        }
        let mut depth = 0;
        let mut k = j;
        while k < bytes.len() {
            match bytes[k] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        out.push(text[j + 1..k.min(bytes.len())].to_string());
        i = k.min(bytes.len());
    }
    out
}

/// Whether a cfg predicate names the platform: `unix`, `windows`, `target_os` or `target_family`
/// as an atom anywhere inside `any`, `all` or `not`.
fn names_platform(predicate: &str) -> bool {
    predicate
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|atom| matches!(atom, "unix" | "windows" | "target_os" | "target_family"))
}

#[test]
fn cfg_scan_reads_nested_predicates() {
    let src = r#"#[cfg(any(test, not(unix)))] fn a() {} fn b() { if cfg!(target_family = "wasm") {} } #[cfg(feature = "x")] fn c() {}"#;
    let hits: Vec<bool> = cfg_predicates(src)
        .iter()
        .map(|p| names_platform(p))
        .collect();
    assert_eq!(hits, vec![true, true, false]);
}

#[test]
fn platform_code_lives_in_one_crate() {
    for (krate, file) in production_sources() {
        if krate == "brain-platform" {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("std::os::"),
            "{}: std::os outside brain-platform",
            file.display()
        );
        for p in cfg_predicates(&text) {
            assert!(
                !names_platform(&p),
                "{}: cfg({p}) outside brain-platform",
                file.display()
            );
        }
    }
    for entry in fs::read_dir(root().join("crates")).unwrap() {
        let dir = entry.unwrap().path();
        if dir.ends_with("brain-platform") {
            continue;
        }
        let manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        for line in manifest
            .lines()
            .filter(|l| l.trim_start().starts_with("[target."))
        {
            assert!(!names_platform(line), "{}: {line}", dir.display());
        }
    }
}

#[test]
fn file_writes_live_in_the_platform_crate() {
    const WRITES: &[&str] = &[
        "remove_file",
        "remove_dir",
        "fs::rename",
        "fs::write",
        "File::create",
        "OpenOptions",
    ];
    let allowed = root().join("crates/brain-platform/src/fs.rs");
    for (_, file) in production_sources() {
        if file == allowed {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        for w in WRITES {
            assert!(
                !text.contains(w),
                "{}: {w} outside brain-platform's fs.rs",
                file.display()
            );
        }
    }
}

#[test]
fn the_clock_allow_lives_in_clock_rs() {
    let allowed = root().join("crates/brain-platform/src/clock.rs");
    for (_, file) in production_sources() {
        let text = fs::read_to_string(&file).unwrap();
        let has = text.contains("clippy::disallowed_methods");
        assert_eq!(
            has,
            file == allowed,
            "{}: the clock allow belongs in clock.rs alone",
            file.display()
        );
    }
}

#[test]
fn layers_point_down() {
    for entry in fs::read_dir(root().join("crates")).unwrap() {
        let dir = entry.unwrap().path();
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let Some(rank) = LAYERS.iter().position(|l| *l == name) else {
            continue;
        };
        let manifest: toml::Table = fs::read_to_string(dir.join("Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
        let deps = manifest
            .get("dependencies")
            .and_then(|d| d.as_table())
            .cloned()
            .unwrap_or_default();
        for dep in deps.keys() {
            if let Some(dep_rank) = LAYERS.iter().position(|l| l == dep) {
                assert!(
                    dep_rank < rank,
                    "{name} depends on {dep}, which is not below it"
                );
            }
        }
    }
}
