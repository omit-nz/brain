//! Text from notes reaches a terminal only through here. A note is written by whoever wrote it,
//! so its escape sequences could retitle the window, rewrite earlier lines or worse.

use std::iter::Peekable;
use std::str::Chars;

/// `text` with every escape sequence and every C0 and C1 control character removed, except
/// newline and tab.
pub fn terminal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => skip_csi(&mut chars),
                Some(']' | 'P' | 'X' | '^' | '_') => skip_string(&mut chars),
                _ => {}
            },
            '\u{9b}' => skip_csi(&mut chars),
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => skip_string(&mut chars),
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// A control sequence: parameter and intermediate bytes, then one final byte.
fn skip_csi(chars: &mut Peekable<Chars<'_>>) {
    for c in chars.by_ref() {
        if !('\u{20}'..='\u{3f}').contains(&c) {
            return;
        }
    }
}

/// An operating-system command or other string, ended by BEL or the string terminator.
fn skip_string(chars: &mut Peekable<Chars<'_>>) {
    while let Some(c) = chars.next() {
        match c {
            '\u{7}' | '\u{9c}' => return,
            '\u{1b}' if chars.peek() == Some(&'\\') => {
                chars.next();
                return;
            }
            _ => {}
        }
    }
}
