//! The strict parser: a line is a simple command only if it holds no shell syntax at all.

/// Why a command line is not a *simple command*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Complex {
    Empty,
    Control,
    Operator(char),
    Expansion(char),
    Backslash,
    UnterminatedQuote,
    Assignment,
    Keyword(String),
    NonAscii,
}

impl Complex {
    pub fn describe(&self) -> String {
        match self {
            Complex::Empty => "empty command".into(),
            Complex::Control => "control characters".into(),
            Complex::Operator(c) => format!("shell operator `{c}`"),
            Complex::Expansion(c) => format!("shell expansion or special character `{c}`"),
            Complex::Backslash => "backslash".into(),
            Complex::UnterminatedQuote => "unterminated quote".into(),
            Complex::Assignment => "variable assignment".into(),
            Complex::Keyword(k) => format!("shell keyword or builtin `{k}`"),
            Complex::NonAscii => "non-ASCII characters outside quotes".into(),
        }
    }
}

/// Words that are keywords or special builtins in at least one of bash, zsh and fish.
const KEYWORDS: &[&str] = &[
    ".",
    ":",
    "if",
    "then",
    "else",
    "elif",
    "fi",
    "for",
    "while",
    "until",
    "do",
    "done",
    "case",
    "esac",
    "function",
    "select",
    "time",
    "coproc",
    "in",
    "[[",
    "and",
    "or",
    "not",
    "begin",
    "end",
    "switch",
    "command",
    "builtin",
    "exec",
    "noglob",
    "nocorrect",
    "eval",
    "source",
    "alias",
    "unalias",
    "set",
    "unset",
    "export",
    "cd",
    "exit",
    "return",
    "trap",
    "wait",
    "kill",
    "fg",
    "bg",
    "jobs",
    "disown",
    "local",
    "declare",
    "typeset",
];

/// The strict subset: the intersection of what bash, zsh and fish parse identically as plain words
/// (plain words, literal single and double quotes, spaces and tabs). Anything else is *complex*: it is
/// never pre-approved and never run by `exec`.
pub fn parse_strict(s: &str) -> Result<Vec<String>, Complex> {
    let mut argv: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            ' ' | '\t' => {
                if in_word {
                    argv.push(std::mem::take(&mut cur));
                    in_word = false;
                    quoted = false;
                }
            }
            '\'' | '"' => {
                in_word = true;
                quoted = true;
                let q = c;
                loop {
                    match it.next() {
                        None => return Err(Complex::UnterminatedQuote),
                        Some(d) if d == q => break,
                        Some('\\') => return Err(Complex::Backslash),
                        Some(d) if d.is_control() => return Err(Complex::Control),
                        Some(d @ ('$' | '`' | '!')) if q == '"' => {
                            return Err(Complex::Expansion(d))
                        }
                        Some(d) => cur.push(d),
                    }
                }
            }
            // `~` and `#` are literal in the middle of a word, but not at its start, after `=`/`:`, or next to a quote
            '~' | '#' if in_word && !quoted && !cur.is_empty() && !cur.ends_with(['=', ':']) => {
                cur.push(c)
            }
            c if c.is_ascii_alphanumeric() || "_./:,+-@%=".contains(c) => {
                in_word = true;
                cur.push(c);
            }
            c if c.is_control() => return Err(Complex::Control),
            '\\' => return Err(Complex::Backslash),
            '$' | '`' | '~' | '*' | '?' | '[' | ']' | '{' | '}' | '#' | '!' => {
                return Err(Complex::Expansion(c))
            }
            '|' | '&' | ';' | '<' | '>' | '(' | ')' => return Err(Complex::Operator(c)),
            c if !c.is_ascii() => return Err(Complex::NonAscii),
            c => return Err(Complex::Operator(c)),
        }
    }
    if in_word {
        argv.push(cur);
    }
    let first = argv.first().ok_or(Complex::Empty)?;
    if first.contains('=') {
        return Err(Complex::Assignment);
    }
    if first.starts_with('%') {
        return Err(Complex::Expansion('%'));
    }
    if KEYWORDS.contains(&first.as_str()) {
        return Err(Complex::Keyword(first.clone()));
    }
    // zsh expands `=cmd` in any word
    if argv.iter().any(|w| w.starts_with('=')) {
        return Err(Complex::Expansion('='));
    }
    Ok(argv)
}
