//! Notes from a learning-loop vault. The copy is the source byte for byte but for the lines the
//! import adds: `visibility` when none is declared, and `owner` and `review` outside the inbox
//! when the note has none.

use brain_note::{Entry, Frontmatter, Note, Place, Skip, Split, Vault, rel_key};

use super::{Source, Written};

pub fn sources(src: &Vault, skipped: &mut Vec<(String, Skip)>) -> Vec<Source> {
    let mut out = Vec::new();
    for entry in src.walk() {
        match entry {
            Entry::Skipped { rel, why } => skipped.push((format!("vault/{}", rel_key(&rel)), why)),
            Entry::Note { rel, .. } => match src.read(&rel) {
                Ok(raw) => out.push(Source {
                    key: format!("vault/{}", rel_key(&rel)),
                    dst_rel: rel,
                    raw,
                    memory: false,
                    from: None,
                }),
                Err(why) => skipped.push((format!("vault/{}", rel_key(&rel)), why)),
            },
        }
    }
    out
}

/// The copy of a vault note. Broken frontmatter is copied as it is: brain reads it as private
/// and never writes into it. A source with no id gets `prior_id`, the id its copy was given
/// before, so updating it keeps the copy's id, or else a fresh one, so the copy is final as
/// written and no index has to write into it.
pub fn to_note(s: &Source, w: &Written<'_>, prior_id: Option<&str>) -> anyhow::Result<String> {
    let mut split = Split::parse(&s.raw);
    let crlf = split
        .body
        .find('\n')
        .is_some_and(|i| split.body[..i].ends_with('\r'));
    let eol = if crlf { "\r\n" } else { "\n" };
    if split.fm.is_err() {
        return Ok(s.raw.clone());
    }
    let id = Note::parse(&s.dst_rel, &s.raw, Place::Kept)
        .id
        .map(|u| u.to_string());
    let mut adds: Vec<(&str, String)> = Vec::new();
    {
        let fm = split.fm.as_ref().ok().and_then(Option::as_ref);
        let has = |k: &str| fm.is_some_and(|f| f.get(k).is_some() || f.misplaced(k));
        if !has("id") {
            let fresh = || uuid::Uuid::now_v7().to_string();
            adds.push(("id", prior_id.map_or_else(fresh, str::to_string)));
        }
        if !has("visibility") {
            adds.push(("visibility", w.visibility(&s.dst_rel).into()));
        }
        if w.layout.place(&s.dst_rel) != Place::Inbox {
            if !has("owner") {
                adds.push(("owner", w.me.to_string()));
            }
            if !has("review") {
                let seed = id.clone().unwrap_or_else(|| s.key.clone());
                adds.push(("review", w.review_date(&seed).to_string()));
            }
        }
    }
    if adds.is_empty() {
        return Ok(s.raw.clone());
    }
    let unsafe_value = |k: &str| anyhow::anyhow!("{}: {k} cannot hold a line break", s.key);
    let fm = match split.fm.as_mut() {
        Ok(Some(fm)) => fm,
        _ => {
            let (k, v) = adds.remove(0);
            split.fm = Ok(Some(
                Frontmatter::new_with(k, &v, eol).map_err(|_| unsafe_value(k))?,
            ));
            split
                .fm
                .as_mut()
                .ok()
                .and_then(Option::as_mut)
                .expect("just set")
        }
    };
    for (k, v) in adds {
        fm.set(k, &v).map_err(|_| unsafe_value(k))?;
    }
    Ok(split.render())
}
