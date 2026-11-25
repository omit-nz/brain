//! Scans of the repo's own source: platform code, file writes and the clock each live in one place,
//! and crates depend only downward. Each scan is a function over files read in, so a fixture can
//! show it fails on a violation.

use std::fs;
use std::path::{Path, PathBuf};

/// The crates in layer order; each may depend only on crates before it.
const LAYERS: &[&str] = &[
    "brain-report",
    "brain-platform",
    "brain-note",
    "brain-index",
    "brain",
];
const DB_RS: &str = "crates/brain-index/src/db.rs";

const PLATFORM: &str = "brain-platform";
const FS_RS: &str = "crates/brain-platform/src/fs.rs";
const CLOCK_RS: &str = "crates/brain-platform/src/clock.rs";

/// One production source file: its crate, its path from the repo root with `/`, and its text.
struct Source {
    krate: String,
    path: String,
    text: String,
}

fn src(krate: &str, path: &str, text: &str) -> Source {
    Source {
        krate: krate.into(),
        path: path.into(),
        text: text.into(),
    }
}

fn manifest(name: &str, text: &str) -> (String, toml::Table) {
    (name.into(), text.parse().unwrap())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn crate_dirs() -> Vec<(String, PathBuf)> {
    fs::read_dir(root().join("crates"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .map(|p| (p.file_name().unwrap().to_string_lossy().to_string(), p))
        .collect()
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
fn repo_sources() -> Vec<Source> {
    let root = root();
    let mut out = Vec::new();
    for (name, dir) in crate_dirs() {
        let mut files = Vec::new();
        rs_files(&dir.join("src"), &mut files);
        for f in files {
            let rel = f.strip_prefix(&root).unwrap();
            let rel: Vec<_> = rel.iter().map(|c| c.to_string_lossy()).collect();
            out.push(src(&name, &rel.join("/"), &fs::read_to_string(&f).unwrap()));
        }
    }
    assert!(!out.is_empty(), "no production sources found");
    out
}

fn repo_manifests() -> Vec<(String, toml::Table)> {
    crate_dirs()
        .into_iter()
        .map(|(name, dir)| {
            let text = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
            manifest(&name, &text)
        })
        .collect()
}

/// The predicate inside each `cfg(`, `cfg!(` or the first argument of `cfg_attr(`.
fn cfg_predicates(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = text[i..].find("cfg") {
        let start = i + at;
        i = start + 3;
        if start > 0 && ident(bytes[start - 1]) {
            continue;
        }
        let rest = &text[i..];
        let (open, attr) = if rest.starts_with('(') {
            (i, false)
        } else if rest.starts_with("!(") {
            (i + 1, false)
        } else if rest.starts_with("_attr(") {
            (i + 5, true)
        } else {
            continue;
        };
        let mut depth = 0;
        let mut end = bytes.len();
        let mut first_comma = None;
        for (k, &b) in bytes.iter().enumerate().skip(open) {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = k;
                        break;
                    }
                }
                b',' if depth == 1 && first_comma.is_none() => first_comma = Some(k),
                _ => {}
            }
        }
        let close = if attr {
            first_comma.unwrap_or(end)
        } else {
            end
        };
        out.push(text[open + 1..close].to_string());
        i = end;
    }
    out
}

#[derive(PartialEq)]
enum Tok {
    Ident(String),
    Str,
    Punct(char),
}

fn tokens(predicate: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut chars = predicate.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '"' {
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        chars.next();
                    }
                    '"' => break,
                    _ => {}
                }
            }
            out.push(Tok::Str);
        } else if c.is_alphanumeric() || c == '_' {
            let mut s = c.to_string();
            while let Some(&n) = chars.peek().filter(|n| n.is_alphanumeric() || **n == '_') {
                s.push(n);
                chars.next();
            }
            out.push(Tok::Ident(s));
        } else {
            out.push(Tok::Punct(c));
        }
    }
    out
}

/// Whether a cfg predicate names the platform: `unix`, `windows`, `target_os` or `target_family`
/// as an option name anywhere inside `any`, `all` or `not`. A string value is never a name, and a
/// predicate the reader cannot follow counts as platform code.
fn names_platform(predicate: &str) -> bool {
    fn pred(t: &[Tok], i: &mut usize) -> Option<bool> {
        let Some(Tok::Ident(name)) = t.get(*i) else {
            return None;
        };
        *i += 1;
        match t.get(*i) {
            Some(Tok::Punct('(')) => {
                *i += 1;
                let mut hit = false;
                loop {
                    if t.get(*i) == Some(&Tok::Punct(')')) {
                        *i += 1;
                        return Some(hit);
                    }
                    hit |= pred(t, i)?;
                    if t.get(*i) == Some(&Tok::Punct(',')) {
                        *i += 1;
                    }
                }
            }
            Some(Tok::Punct('=')) => {
                *i += 1;
                (t.get(*i) == Some(&Tok::Str)).then(|| {
                    *i += 1;
                    is_platform(name)
                })
            }
            _ => Some(is_platform(name)),
        }
    }
    fn is_platform(name: &str) -> bool {
        matches!(name, "unix" | "windows" | "target_os" | "target_family")
    }
    let t = tokens(predicate);
    let mut i = 0;
    match pred(&t, &mut i) {
        Some(hit) if i == t.len() => hit,
        _ => true,
    }
}

fn platform_code(sources: &[Source]) -> Vec<String> {
    let mut out = Vec::new();
    for s in sources.iter().filter(|s| s.krate != PLATFORM) {
        if s.text.contains("std::os::") {
            out.push(format!("{}: std::os outside {PLATFORM}", s.path));
        }
        for p in cfg_predicates(&s.text) {
            if names_platform(&p) {
                out.push(format!("{}: cfg({p}) outside {PLATFORM}", s.path));
            }
        }
    }
    out
}

fn platform_manifests(manifests: &[(String, toml::Table)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, m) in manifests.iter().filter(|(n, _)| n != PLATFORM) {
        for key in m
            .get("target")
            .and_then(|t| t.as_table())
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
        {
            if names_platform(key.trim_start_matches("cfg(").trim_end_matches(')')) {
                out.push(format!("{name}: [target.'{key}'] outside {PLATFORM}"));
            }
        }
    }
    out
}

/// Every std and tempfile route that changes the disk. `rename(` also catches `fs::rename(` but
/// not the platform crate's `rename_within(`.
const WRITES: &[&str] = &[
    "fs::write",
    "fs::copy",
    "create_dir(",
    "create_dir_all(",
    "hard_link",
    "soft_link",
    "symlink_file",
    "symlink_dir",
    "set_permissions",
    "File::create",
    "File::options",
    "OpenOptions",
    "persist(",
    "remove_file",
    "remove_dir",
    "rename(",
];

fn file_writes(sources: &[Source]) -> Vec<String> {
    let mut out = Vec::new();
    for s in sources.iter().filter(|s| s.path != FS_RS) {
        for w in WRITES {
            if s.text.contains(w) {
                out.push(format!("{}: {w} outside {FS_RS}", s.path));
            }
        }
    }
    out
}

/// One function opens every SQLite database, so every store gets the same privacy, WAL and
/// migration rules.
fn connection_openers(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .filter(|s| s.path != DB_RS && s.text.contains("Connection::open"))
        .map(|s| format!("{}: Connection::open outside {DB_RS}", s.path))
        .collect()
}

#[test]
fn one_connection_opener() {
    assert_eq!(connection_openers(&repo_sources()), Vec::<String>::new());
    let stray = [src(
        "brain",
        "crates/brain/src/usage.rs",
        "let c = rusqlite::Connection::open(path)?;",
    )];
    assert_eq!(connection_openers(&stray).len(), 1);
    let home = [src("brain-index", DB_RS, "Connection::open(path)")];
    assert_eq!(connection_openers(&home), Vec::<String>::new());
}

fn clock_allows(sources: &[Source]) -> Vec<String> {
    sources
        .iter()
        .filter(|s| s.text.contains("clippy::disallowed_methods") != (s.path == CLOCK_RS))
        .map(|s| format!("{}: the clock allow belongs in {CLOCK_RS} alone", s.path))
        .collect()
}

/// Every crate is in `LAYERS` (or is this rules crate), and every dependency of any kind, renamed
/// or per-target, points to a crate below it.
fn layers(manifests: &[(String, toml::Table)]) -> Vec<String> {
    const KINDS: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];
    let mut out = Vec::new();
    for (name, m) in manifests {
        let Some(rank) = LAYERS.iter().position(|l| l == name) else {
            if name != "brain-repo-rules" {
                out.push(format!("{name} is not in LAYERS"));
            }
            continue;
        };
        let targets = m.get("target").and_then(|t| t.as_table());
        let tables = std::iter::once(m).chain(
            targets
                .into_iter()
                .flat_map(|t| t.values().filter_map(|v| v.as_table())),
        );
        for table in tables {
            for kind in KINDS {
                let deps = table.get(*kind).and_then(|d| d.as_table());
                for (key, spec) in deps.into_iter().flatten() {
                    let dep = spec.get("package").and_then(|p| p.as_str()).unwrap_or(key);
                    if LAYERS
                        .iter()
                        .position(|l| *l == dep)
                        .is_some_and(|d| d >= rank)
                    {
                        out.push(format!("{name} depends on {dep}, which is not below it"));
                    }
                }
            }
        }
    }
    out
}

#[test]
fn cfg_scan_reads_nested_predicates() {
    let src = r#"#[cfg(any(test, not(unix)))] fn a() {} fn b() { if cfg!(target_family = "wasm") {} }
        #[cfg(feature = "x")] fn c() {} #[cfg(feature = "windows")] fn d() {}
        #[cfg_attr(unix, derive(Debug))] struct E; #[cfg_attr(test, derive(Debug))] struct F;"#;
    let hits: Vec<bool> = cfg_predicates(src)
        .iter()
        .map(|p| names_platform(p))
        .collect();
    assert_eq!(hits, vec![true, true, false, false, true, false]);
}

#[test]
fn platform_code_lives_in_one_crate() {
    assert_eq!(platform_code(&repo_sources()), Vec::<String>::new());
    assert_eq!(platform_manifests(&repo_manifests()), Vec::<String>::new());
}

#[test]
fn the_platform_scan_catches_os_code_elsewhere() {
    let fixture = [
        src(
            "brain-note",
            "crates/brain-note/src/a.rs",
            "use std::os::unix::fs::PermissionsExt;",
        ),
        src(
            "brain-note",
            "crates/brain-note/src/b.rs",
            "#[cfg(all(test, target_os = \"macos\"))] fn b() {}",
        ),
        src(
            "brain-platform",
            "crates/brain-platform/src/c.rs",
            "#[cfg(unix)] use std::os::unix;",
        ),
    ];
    let found = platform_code(&fixture);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().all(|f| f.starts_with("crates/brain-note/")));
}

#[test]
fn the_platform_scan_catches_a_target_table_elsewhere() {
    let fixture = [
        manifest(
            "brain-note",
            "[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n",
        ),
        manifest(
            "brain-index",
            "[target.x86_64-pc-windows-msvc.dependencies]\nwindows-sys = \"0.59\"\n",
        ),
        manifest(
            "brain-platform",
            "[target.'cfg(windows)'.dependencies]\nwindows-sys = \"0.59\"\n",
        ),
    ];
    let found = platform_manifests(&fixture);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(!found.iter().any(|f| f.starts_with(PLATFORM)));
}

#[test]
fn file_writes_live_in_the_platform_crate() {
    assert_eq!(file_writes(&repo_sources()), Vec::<String>::new());
}

#[test]
fn the_write_scan_catches_every_route_to_disk() {
    let routes = [
        "std::fs::write(p, b)",
        "std::fs::copy(a, b)",
        "std::fs::create_dir_all(p)",
        "std::fs::create_dir(p)",
        "std::fs::hard_link(a, b)",
        "std::fs::set_permissions(p, perm)",
        "std::fs::File::options().write(true).open(p)",
        "OpenOptions::new().append(true).open(p)",
        "tmp.persist(p)",
        "std::fs::remove_file(p)",
        "rename(a, b)",
    ];
    for r in routes {
        let fixture = [src("brain-note", "crates/brain-note/src/a.rs", r)];
        assert!(
            !file_writes(&fixture).is_empty(),
            "{r} passed the write scan"
        );
    }
    let fine = [
        src("brain-platform", FS_RS, "std::fs::write(p, b)"),
        src(
            "brain-note",
            "crates/brain-note/src/a.rs",
            "brain_platform::fs::rename_within(r, a, b); create_dir_private(p); is_symlink()",
        ),
    ];
    assert_eq!(file_writes(&fine), Vec::<String>::new());
}

#[test]
fn the_clock_allow_lives_in_clock_rs() {
    assert_eq!(clock_allows(&repo_sources()), Vec::<String>::new());
}

#[test]
fn the_clock_scan_catches_a_stray_or_missing_allow() {
    let allow = "#[allow(clippy::disallowed_methods)] fn now() {}";
    let stray = [
        src("brain-platform", CLOCK_RS, allow),
        src("brain-note", "crates/brain-note/src/a.rs", allow),
    ];
    assert_eq!(clock_allows(&stray).len(), 1);
    let missing = [src("brain-platform", CLOCK_RS, "fn now() {}")];
    assert_eq!(clock_allows(&missing).len(), 1);
}

/// clippy.toml's `allow-invalid` hides a misspelled path, so the list is read here: every clock
/// read the spec names must be disallowed outside clock.rs.
#[test]
fn clippy_disallows_every_clock_read() {
    let clippy: toml::Table = fs::read_to_string(root().join("clippy.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let paths: Vec<&str> = clippy["disallowed-methods"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m.get("path").and_then(|p| p.as_str()))
        .collect();
    for read in [
        "std::time::SystemTime::now",
        "jiff::Timestamp::now",
        "jiff::Zoned::now",
    ] {
        assert!(
            paths.contains(&read),
            "clippy.toml does not disallow {read}"
        );
    }
}

#[test]
fn layers_point_down() {
    assert_eq!(layers(&repo_manifests()), Vec::<String>::new());
}

#[test]
fn the_layer_scan_catches_an_upward_or_unlisted_dependency() {
    let cases = [
        manifest(
            "brain-platform",
            "[dependencies]\nbrain-note = { path = \"../brain-note\" }\n",
        ),
        manifest(
            "brain-platform",
            "[dev-dependencies]\nbrain-note = { path = \"../brain-note\" }\n",
        ),
        manifest(
            "brain-note",
            "[build-dependencies]\nbrain-index = { path = \"../brain-index\" }\n",
        ),
        manifest(
            "brain-note",
            "[dependencies]\nidx = { package = \"brain-index\", path = \"../brain-index\" }\n",
        ),
        manifest(
            "brain-note",
            "[target.'cfg(test)'.dependencies]\nbrain = { path = \"../brain\" }\n",
        ),
        manifest("brain-notes", "[dependencies]\n"),
    ];
    for (name, m) in &cases {
        assert!(
            !layers(&[(name.clone(), m.clone())]).is_empty(),
            "{name}: {m} passed the layer scan"
        );
    }
    let fine = [
        manifest(
            "brain-note",
            "[dependencies]\nbrain-platform = { path = \"../brain-platform\" }\n",
        ),
        manifest("brain-repo-rules", "[dev-dependencies]\ntoml = \"0.9\"\n"),
    ];
    assert_eq!(layers(&fine), Vec::<String>::new());
}
