//! Cleaning text that a program wrote before it is shown or sent: terminal codes out, length capped.

/// Remove overstrike pairs (`c\x08c`, `_\x08c`), ANSI escapes and blank-line runs.
pub fn clean_man(raw: &str) -> String {
    let mut chars: Vec<char> = Vec::with_capacity(raw.len());
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{8}' => {
                chars.pop();
            }
            '\u{1b}' => {
                if it.peek() == Some(&'[') {
                    it.next();
                    for n in it.by_ref() {
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            }
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => chars.push(c),
        }
    }
    let mut out = String::new();
    let mut blank = 0;
    for line in chars.into_iter().collect::<String>().lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

// ---- the read tool -----------------------------------------------------------------------------------------

/// Remove escape sequences (colors, cursor moves, titles) and other control characters; keep `\n` and `\t`.
pub(super) fn strip_terminal_codes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.peek() {
                Some('[') => {
                    it.next();
                    for n in it.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    it.next();
                    while let Some(n) = it.next() {
                        if n == '\u{7}' {
                            break;
                        }
                        if n == '\u{1b}' {
                            it.next(); // the `\` of the string terminator
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            },
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
