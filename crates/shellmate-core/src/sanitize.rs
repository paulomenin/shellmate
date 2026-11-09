//! One sanitizer for everything shown on the terminal that did not come from us
//! (model text, tool output, command text). Control sequences become visible text.

/// Escape a single character for display. `None` means "keep as is".
pub fn escape_char(c: char) -> Option<String> {
    match c {
        '\\' => Some("\\\\".to_string()),
        '\n' => Some("\\n".to_string()),
        '\t' => Some("\\t".to_string()),
        c if c.is_control() => Some(format!("\\u{{{:x}}}", c as u32)),
        // bidi overrides/isolates, zero-width characters, BOM
        '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{feff}' => Some(format!("\\u{{{:x}}}", c as u32)),
        _ => None,
    }
}

/// Escape a whole string on a single line (newlines become `\n`).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match escape_char(c) {
            Some(e) => out.push_str(&e),
            None => out.push(c),
        }
    }
    out
}

/// Escape control characters and invisible or direction-changing characters, but keep what a person
/// reading an error message expects to see unchanged: line breaks, tabs and backslashes.
pub fn escape_keep_layout(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\t' | '\\' => out.push(c),
            c => match escape_char(c) {
                Some(e) => out.push_str(&e),
                None => out.push(c),
            },
        }
    }
    out
}

/// True when `c` may appear in a command that is placed into the user's input buffer.
pub fn is_buffer_safe(c: char) -> bool {
    c == '\n' || c == '\t' || (!c.is_control() && escape_char(c).is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_text_is_inert() {
        let s = escape("a\u{1b}[2J\u{1b}]0;pwned\u{7}\u{202e}rm\u{0}\\");
        assert!(!s.chars().any(|c| c.is_control()), "{s:?}");
        assert!(s.contains("\\u{1b}[2J") && s.contains("\\u{202e}") && s.ends_with("\\\\"));
    }

    #[test]
    fn plain_unicode_is_kept() {
        assert_eq!(escape("café 日本語 🙂"), "café 日本語 🙂");
    }

    #[test]
    fn buffer_safety() {
        assert!(is_buffer_safe('a') && is_buffer_safe('\n') && is_buffer_safe('é'));
        assert!(!is_buffer_safe('\u{1b}') && !is_buffer_safe('\u{202e}') && !is_buffer_safe('\r'));
    }

    #[test]
    fn newlines_and_tabs_get_short_escapes() {
        assert_eq!(escape("a\nb\tc"), "a\\nb\\tc");
        assert_eq!(escape_char('\n').as_deref(), Some("\\n"));
        assert_eq!(escape_char('\t').as_deref(), Some("\\t"));
    }
}
