//! Claude Code auto-memory: `<projects>/*/memory/*.md`, each a small note with `name`,
//! `description` and a `type` either at the top or under `metadata`.

use std::fs;
use std::path::{Path, PathBuf};

use brain_note::{Frontmatter, Kind, MAX_NOTE_BYTES, Skip, Split, Value, text_of};
use brain_platform::fs::is_link;

use super::{Source, Written};

/// Every memory file to import, in a stable order. A project or memory folder that is a link is
/// skipped as one, so a tree reached twice is read once.
pub fn sources(
    root: &Path,
    dst_folder: &Path,
    skipped: &mut Vec<(String, Skip)>,
) -> anyhow::Result<Vec<Source>> {
    let mut out = Vec::new();
    let mut projects: Vec<PathBuf> = fs::read_dir(root)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    projects.sort();
    for project in projects {
        let name = project
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let dir = project.join("memory");
        if is_link(&project) || is_link(&dir) {
            skipped.push((format!("memory/{name}"), Skip::Link));
            continue;
        }
        if !dir.is_dir() {
            continue;
        }
        let mut files: Vec<PathBuf> = fs::read_dir(&dir)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        files.sort();
        for file in files {
            let fname = file
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let wanted =
                fname.ends_with(".md") && fname != "MEMORY.md" && !fname.starts_with(['_', '.']);
            if !wanted || file.is_dir() {
                continue;
            }
            let key = format!("memory/{name}/{fname}");
            if is_link(&file) {
                skipped.push((key, Skip::Link));
                continue;
            }
            let read = fs::metadata(&file).and_then(|m| {
                if m.len() > MAX_NOTE_BYTES {
                    Ok(Err(Skip::TooLarge(m.len())))
                } else {
                    fs::read(&file).map(|b| String::from_utf8(b).map_err(|_| Skip::NotUtf8))
                }
            });
            match read {
                Ok(Ok(raw)) => out.push(Source {
                    key,
                    dst_rel: dst_folder.join(&name).join(&fname),
                    raw,
                    memory: true,
                }),
                Ok(Err(why)) => skipped.push((key, why)),
                Err(e) => skipped.push((key, Skip::Unreadable(e.to_string()))),
            }
        }
    }
    Ok(out)
}

fn kind_of(t: &str) -> Kind {
    match t.trim() {
        "feedback" => Kind::Rule,
        "project" => Kind::History,
        _ => Kind::Knowledge,
    }
}

fn text(v: Option<Value>) -> Option<String> {
    text_of(v)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// The memory as a brain note: a new block with its id, title, kind, private visibility, owner
/// and review date, and its description as the first body line.
pub fn to_note(s: &Source, id: &str, w: &Written<'_>) -> anyhow::Result<String> {
    let split = Split::parse(&s.raw);
    let fm = split.fm.as_ref().ok().and_then(Option::as_ref);
    let get = |k: &str| fm.and_then(|f| text(f.get(k)));
    let kind = get("type")
        .or_else(|| fm.and_then(|f| text(f.get_path(&["metadata", "type"]))))
        .map_or(Kind::Knowledge, |t| kind_of(&t));
    let stem = s
        .dst_rel
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let title = get("name").unwrap_or(stem);
    let body = match (&split.fm, get("description")) {
        (Ok(_), Some(d)) => format!("{d}\n\n{}", split.body.trim_start_matches(['\n', '\r'])),
        (Ok(_), None) => split.body.to_string(),
        (Err(_), _) => s.raw.clone(),
    };
    let unsafe_value = |k: &str| anyhow::anyhow!("{}: {k} cannot hold a line break", s.key);
    let mut out = Frontmatter::new_with("id", id, "\n").map_err(|_| unsafe_value("id"))?;
    let fields = [
        ("title", title),
        ("kind", kind.as_str().to_string()),
        ("visibility", "private".to_string()),
        ("owner", w.me.to_string()),
        ("review", w.review_date(id).to_string()),
    ];
    for (k, v) in fields {
        out.set(k, &v).map_err(|_| unsafe_value(k))?;
    }
    Ok(Split {
        bom: false,
        fm: Ok(Some(out)),
        body: &body,
    }
    .render())
}
