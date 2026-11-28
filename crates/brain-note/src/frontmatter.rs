//! Frontmatter held as the lines the author wrote. Reading parses only the key asked for, and
//! writing touches only the key set, so comments, order and unknown keys survive byte for byte.

use std::ops::Range;

use yaml_rust2::YamlLoader;

const BOM: &str = "\u{feff}";

#[derive(Debug, PartialEq, Eq)]
pub enum FmError {
    /// An opening `---` with no closing fence.
    Unclosed,
    /// A key or value that would write a second key, or a key already written twice.
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
    /// The key appears more than once at column zero.
    Conflict,
    /// A block scalar (`|`, `>`), a flow mapping or a nested mapping: not text this reader takes.
    Other,
}

#[derive(Debug, Clone)]
pub struct Frontmatter {
    open: String,
    lines: Vec<String>,
    close: String,
}

pub struct Split<'a> {
    pub bom: bool,
    pub fm: Result<Option<Frontmatter>, FmError>,
    pub body: &'a str,
}

/// Splits `text` into lines that keep their line ends.
fn lines_of(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

/// `---` alone on its line; trailing spaces and tabs an editor left behind still count.
fn is_fence(line: &str) -> bool {
    line.trim_end() == "---"
}

impl<'a> Split<'a> {
    pub fn parse(raw: &'a str) -> Split<'a> {
        let bom = raw.starts_with(BOM);
        let text = &raw[if bom { BOM.len() } else { 0 }..];
        let lines = lines_of(text);
        let Some(first) = lines.first().filter(|l| is_fence(l) && l.ends_with('\n')) else {
            return Split {
                bom,
                fm: Ok(None),
                body: text,
            };
        };
        let Some(end) = lines
            .iter()
            .skip(1)
            .position(|l| is_fence(l))
            .map(|i| i + 1)
        else {
            return Split {
                bom,
                fm: Err(FmError::Unclosed),
                body: raw,
            };
        };
        let body_at: usize = lines[..=end].iter().map(|l| l.len()).sum();
        let fm = Frontmatter {
            open: first.to_string(),
            lines: lines[1..end].iter().map(|l| l.to_string()).collect(),
            close: lines[end].to_string(),
        };
        Split {
            bom,
            fm: Ok(Some(fm)),
            body: &text[body_at..],
        }
    }

    /// The note's text again: byte for byte what was parsed, plus any key that was set.
    pub fn render(&self) -> String {
        let mut out = String::new();
        match &self.fm {
            Err(_) => return self.body.to_string(),
            Ok(fm) => {
                if self.bom {
                    out.push_str(BOM);
                }
                if let Some(fm) = fm {
                    out.push_str(&fm.render());
                }
            }
        }
        out.push_str(self.body);
        out
    }
}

/// `key:` at column zero, returning what follows the colon.
fn value_after<'l>(line: &'l str, key: &str) -> Option<&'l str> {
    line.strip_prefix(key)?.strip_prefix(':')
}

/// Whether `line` names `key` in any YAML spelling: indented, quoted, spaced before the colon,
/// in another case, inside a flow mapping, as a list item's key or as an explicit `? key`.
fn names_key(line: &str, key: &str) -> bool {
    line.split([',', '{']).any(|seg| {
        let seg = seg.trim_start();
        let (seg, explicit) = match seg.strip_prefix('?') {
            Some(rest) => (rest, true),
            None => (seg, false),
        };
        let seg = seg.trim_start().trim_start_matches("- ");
        let k = match seg.split_once(':') {
            Some((k, _)) => k,
            None if explicit => seg,
            None => return false,
        };
        unquote(k.trim()).eq_ignore_ascii_case(key)
    })
}

/// Lines that continue the key on the line before: indented lines and block list items.
fn continues(line: &str) -> bool {
    line.starts_with([' ', '\t']) || line.starts_with("- ") || line.trim_end() == "-"
}

fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

fn parse_value(rest: &str, following: &[String]) -> Value {
    let v = rest.trim();
    if v.starts_with(['|', '>', '{']) {
        return Value::Other;
    }
    if let Some(inner) = v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Value::List(
            inner
                .split(',')
                .map(|s| unquote(s.trim()).to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        );
    }
    if v.is_empty() {
        let block: Vec<&str> = following
            .iter()
            .map(|l| l.trim_end_matches(['\r', '\n']))
            .take_while(|l| continues(l))
            .collect();
        let items: Vec<String> = block
            .iter()
            .filter_map(|l| l.trim_start().strip_prefix("- "))
            .map(|s| unquote(s.trim()).to_string())
            .collect();
        if !block.is_empty() {
            return if items.len() == block.len() {
                Value::List(items)
            } else {
                Value::Other
            };
        }
    }
    // YAML folds an indented next line into a plain value; a value it would read differently is
    // in doubt.
    if following
        .first()
        .is_some_and(|l| continues(l.trim_end_matches(['\r', '\n'])))
    {
        return Value::Other;
    }
    Value::Text(v.to_string())
}

/// A value YAML would read as something other than this text: a comment, an indicator, a
/// mapping, or a bool, null or number.
fn needs_quotes(v: &str) -> bool {
    const RESERVED: &[&str] = &[
        "true", "false", "yes", "no", "on", "off", "y", "n", "null", "~",
    ];
    v.is_empty()
        || v.contains(": ")
        || v.contains(" #")
        || v.contains('\t')
        || v.ends_with(':')
        || v.starts_with([
            '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', '{', '[', '-', ' ', '?', ',',
            '+', '.',
        ])
        || v.parse::<f64>().is_ok()
        || v.chars()
            .all(|c| c.is_ascii_digit() || c == '_' || c == ':')
        || v.starts_with("0x")
        || v.starts_with("0o")
        || v.ends_with(' ')
        || RESERVED.iter().any(|r| v.eq_ignore_ascii_case(r))
}

impl Frontmatter {
    pub fn get(&self, key: &str) -> Option<Value> {
        let hits: Vec<usize> = (0..self.lines.len())
            .filter(|&i| value_after(&self.lines[i], key).is_some())
            .collect();
        match hits.as_slice() {
            [] => None,
            [i] => {
                let rest = value_after(&self.lines[*i], key)
                    .unwrap()
                    .trim_end_matches(['\r', '\n']);
                Some(parse_value(rest, &self.lines[i + 1..]))
            }
            _ => Some(Value::Conflict),
        }
    }

    /// A key's words, on one line. A value written on the key's own line is read as `get` reads
    /// it; one written over several lines, as YAML reads it. `None` where that is not words: a
    /// list, a mapping, or lines YAML cannot read.
    pub fn words(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            // `key: # note` is a comment and no value.
            Value::Text(t) if !t.starts_with('#') => text_of(Some(Value::Text(t))),
            Value::Conflict => None,
            _ => {
                let docs = YamlLoader::load_from_str(&self.lines.concat()).ok()?;
                let words: Vec<&str> = docs.first()?[key].as_str()?.split_whitespace().collect();
                Some(words.join(" ")).filter(|t| !t.is_empty())
            }
        }
    }

    /// One level of block mapping: `get_path(&["metadata", "type"])`.
    pub fn get_path(&self, path: &[&str]) -> Option<Value> {
        let [parent, child] = path else { return None };
        let start = self
            .lines
            .iter()
            .position(|l| value_after(l, parent).is_some())?;
        self.lines[start + 1..]
            .iter()
            .take_while(|l| l.starts_with(' ') || l.starts_with('\t'))
            .find_map(|l| value_after(l.trim_start(), child))
            .map(|rest| Value::Text(unquote(rest.trim()).to_string()))
    }

    /// The key written any way other than plain `key:` at column zero (indented, quoted, spaced,
    /// in another case, in a flow mapping), where `get` would miss it.
    pub fn misplaced(&self, key: &str) -> bool {
        self.lines
            .iter()
            .any(|l| names_key(l, key) && value_after(l, key).is_none())
    }

    /// Writes `key: value`, replacing the key's line and its continuation lines, or adding one
    /// before the closing fence. Refuses a key that is not a plain name, a value holding a line
    /// break, and a key already written twice.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), FmError> {
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.');
        if key.is_empty() || !key.chars().all(plain) || value.contains(['\n', '\r']) {
            return Err(FmError::Unsafe);
        }
        if self.get(key) == Some(Value::Conflict) {
            return Err(FmError::Unsafe);
        }
        let shown = if needs_quotes(value) {
            format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            value.to_string()
        };
        let eol = if self.open.ends_with("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let line = format!("{key}: {shown}{eol}");
        match self.span(key) {
            Some(at) => {
                self.lines.splice(at, [line]);
            }
            None => self.lines.push(line),
        }
        Ok(())
    }

    /// Takes out `key`'s line and the lines that continue it.
    pub fn remove(&mut self, key: &str) {
        if let Some(at) = self.span(key) {
            self.lines.drain(at);
        }
    }

    /// Whether `key` is written as the one line `key: value`, with nothing added to it.
    pub fn holds_only(&self, key: &str, value: &str) -> bool {
        self.span(key).is_some_and(|at| {
            at.len() == 1
                && self.lines[at.start].trim_end_matches(['\r', '\n']) == format!("{key}: {value}")
        })
    }

    /// Where `key`'s line and the lines that continue it are.
    fn span(&self, key: &str) -> Option<Range<usize>> {
        let start = self
            .lines
            .iter()
            .position(|l| value_after(l, key).is_some())?;
        let more = self.lines[start + 1..]
            .iter()
            .take_while(|l| continues(l))
            .count();
        Some(start..start + 1 + more)
    }

    fn render(&self) -> String {
        let mut out = self.open.clone();
        for l in &self.lines {
            out.push_str(l);
        }
        out.push_str(&self.close);
        out
    }

    /// A block holding only `key: value`, for a note that had no frontmatter, with the line end
    /// `eol` the note already uses.
    pub fn new_with(key: &str, value: &str, eol: &str) -> Result<Frontmatter, FmError> {
        let mut fm = Frontmatter {
            open: format!("---{eol}"),
            lines: Vec::new(),
            close: format!("---{eol}"),
        };
        fm.set(key, value)?;
        Ok(fm)
    }
}

/// A text value as written, with the quotes and escapes `set` adds taken off again.
pub fn text_of(v: Option<Value>) -> Option<String> {
    match v {
        Some(Value::Text(t)) if !t.is_empty() => {
            let inner = unquote(&t);
            if inner.len() + 2 == t.len() && t.starts_with('"') {
                let mut out = String::with_capacity(inner.len());
                let mut chars = inner.chars();
                while let Some(c) = chars.next() {
                    match (c, chars.clone().next()) {
                        ('\\', Some(n @ ('\\' | '"'))) => {
                            out.push(n);
                            chars.next();
                        }
                        _ => out.push(c),
                    }
                }
                Some(out)
            } else {
                Some(inner.to_string())
            }
        }
        _ => None,
    }
}
