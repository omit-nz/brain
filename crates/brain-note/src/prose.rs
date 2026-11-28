//! A note's text apart from its code: what a heading or a marker is looked for in.

/// The lines of `body` outside its fenced code blocks. A fence opens on three or more backticks or
/// tildes, after a list marker or not, and closes on at least as many of the same.
pub fn prose(body: &str) -> impl Iterator<Item = &str> {
    // The open block's marks and the column they sit at.
    let mut block: Option<(&str, usize)> = None;
    body.lines().filter(move |line| {
        let text = line.trim_start();
        let indent = line.len() - text.len();
        if let Some((marks, column)) = block {
            // Four or more columns in, the marks are in a list item or are themselves indented
            // code: either way the block is over where a line falls back left of them.
            if column < 4 || indent >= column || text.is_empty() {
                let text = text.trim_end();
                let closes = text.starts_with(marks)
                    && text.trim_start_matches(&marks[..1]).is_empty()
                    && indent <= column + 3;
                if closes {
                    block = None;
                }
                return false;
            }
        }
        block = opens(line);
        block.is_none()
    })
}

/// The marks `line` opens a code block with, and their column.
fn opens(line: &str) -> Option<(&str, usize)> {
    let mut text = line.trim_start();
    let number = text.trim_start_matches(|c: char| c.is_ascii_digit());
    let item = match number.len() < text.len() {
        true => number.strip_prefix(['.', ')']),
        false => text.strip_prefix(['-', '*', '+']),
    };
    if let Some(rest) = item.filter(|rest| rest.starts_with(' ')) {
        text = rest.trim_start();
    }
    let mark = text.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let after = text.trim_start_matches(mark);
    let marks = &text[..text.len() - after.len()];
    // Three backticks with another later on the line are code inside a sentence.
    (marks.len() >= 3 && !(mark == '`' && after.contains('`')))
        .then_some((marks, line.len() - text.len()))
}
