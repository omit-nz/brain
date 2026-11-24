//! Frontmatter held as the lines the author wrote. Reading parses only the key asked for, and
//! writing touches only the key set, so comments, order and unknown keys survive byte for byte.

const BOM: &str = "\u{feff}";

#[derive(Debug, PartialEq, Eq)]
pub enum FmError {
    /// An opening `---` with no closing fence.
    Unclosed,
    /// A value holding a line break, which would write a second key.
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
    /// The key appears more than once at column zero.
    Conflict,
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

fn is_fence(line: &str) -> bool {
    line.trim_end_matches(['\r', '\n']) == "---"
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
        let items: Vec<String> = following
            .iter()
            .map(|l| l.trim_end_matches(['\r', '\n']))
            .take_while(|l| l.starts_with(' ') || l.starts_with('\t') || l.starts_with('-'))
            .filter_map(|l| l.trim_start().strip_prefix("- "))
            .map(|s| unquote(s.trim()).to_string())
            .collect();
        if !items.is_empty() {
            return Value::List(items);
        }
    }
    Value::Text(v.to_string())
}

fn needs_quotes(v: &str) -> bool {
    v.is_empty()
        || v.contains(": ")
        || v.contains(" #")
        || v.starts_with([
            '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', '{', '[', '-', ' ',
        ])
        || v.ends_with(' ')
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

    /// The key written somewhere other than column zero, where a reader of column-zero keys
    /// would miss it.
    pub fn misplaced(&self, key: &str) -> bool {
        self.lines.iter().any(|l| {
            let t = l.trim_start();
            t.len() < l.len() && value_after(t, key).is_some()
        })
    }

    /// Writes `key: value`, replacing the key's line or adding one before the closing fence.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), FmError> {
        if value.contains(['\n', '\r']) {
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
        match self
            .lines
            .iter()
            .position(|l| value_after(l, key).is_some())
        {
            Some(i) => self.lines[i] = line,
            None => self.lines.push(line),
        }
        Ok(())
    }

    fn render(&self) -> String {
        let mut out = self.open.clone();
        for l in &self.lines {
            out.push_str(l);
        }
        out.push_str(&self.close);
        out
    }

    /// A block holding only `key: value`, for a note that had no frontmatter.
    pub fn new_with(key: &str, value: &str) -> Result<Frontmatter, FmError> {
        let mut fm = Frontmatter {
            open: "---\n".into(),
            lines: Vec::new(),
            close: "---\n".into(),
        };
        fm.set(key, value)?;
        Ok(fm)
    }
}

pub fn text_of(v: Option<Value>) -> Option<String> {
    match v {
        Some(Value::Text(t)) if !t.is_empty() => Some(unquote(&t).to_string()),
        _ => None,
    }
}
