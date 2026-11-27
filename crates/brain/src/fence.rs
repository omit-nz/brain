//! A note's own text on its way to an AI: escaped, so nothing in it can pass for an instruction
//! or close the element that holds it.

use brain_note::Kind;

use crate::out::terminal;

/// Bidi embeddings, overrides and isolates, which reorder what a reader sees, and the tag block,
/// which spells hidden ASCII. Joiners stay: emoji and several scripts need them.
fn hidden(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{E0000}'..='\u{E007F}')
}

/// `&`, `<`, `>` and `"` as entities, after control and hidden characters are stripped.
pub fn escape(text: &str) -> String {
    terminal(text)
        .replace(hidden, "")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn attrs(id: Option<uuid::Uuid>, path: &str, title: &str, kind: Option<Kind>) -> String {
    let mut a = String::new();
    if let Some(id) = id {
        a.push_str(&format!(" id=\"{id}\""));
    }
    a.push_str(&format!(
        " path=\"{}\" title=\"{}\"",
        escape(path),
        escape(title)
    ));
    if let Some(k) = kind {
        a.push_str(&format!(" kind=\"{}\"", k.as_str()));
    }
    a
}

/// A note named without its body.
pub fn fence_ref(id: Option<uuid::Uuid>, path: &str, title: &str, kind: Option<Kind>) -> String {
    format!("<note{} />", attrs(id, path, title, kind))
}
