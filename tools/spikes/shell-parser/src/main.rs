//! Spike: strict "simple command" parser vs shlex.
//! stdin: one JSON string per line. stdout: one JSON per line: argv array or null (rejected).
//! usage: shell-parser-spike strict|shlex
use std::io::BufRead;

#[derive(Debug, PartialEq)]
pub enum Reject {
    Empty, Control, Operator(char), Expansion(char), Backslash, UnterminatedQuote, Assignment, Keyword(String), NonAscii,
}

const KEYWORDS: &[&str] = &[
    ".", ":",
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "function",
    "select", "time", "coproc", "in", "[[", "and", "or", "not", "begin", "end", "switch", "command",
    "builtin", "exec", "noglob", "nocorrect", "eval", "source", "alias", "unalias", "set", "unset", "export",
    "cd", "exit", "return", "trap", "wait", "kill", "fg", "bg", "jobs", "disown", "local", "declare", "typeset",
];

/// Strict subset: the intersection of what bash, zsh and fish parse identically as plain words.
/// Anything else is rejected (=> "complex": always ask, never pre-approve).
pub fn parse_strict(s: &str) -> Result<Vec<String>, Reject> {
    let mut argv: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quoted = false; // current word contains a quoted part
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            ' ' | '\t' => { if in_word { argv.push(std::mem::take(&mut cur)); in_word = false; quoted = false; } }
            '\'' | '"' => {
                in_word = true;
                quoted = true;
                let q = c;
                loop {
                    match it.next() {
                        None => return Err(Reject::UnterminatedQuote),
                        Some(d) if d == q => break,
                        Some('\\') => return Err(Reject::Backslash),
                        Some(d) if d.is_control() => return Err(Reject::Control),
                        Some(d @ ('$' | '`' | '!')) if q == '"' => return Err(Reject::Expansion(d)),
                        Some(d) => cur.push(d),
                    }
                }
            }
            // `~` and `#` are literal in the middle of a word, but not after `=`/`:` (tilde expansion in assignment-like words)
            '~' | '#' if in_word && !quoted && !cur.is_empty() && !cur.ends_with(['=', ':']) => { cur.push(c); }
            c if c.is_ascii_alphanumeric() || "_./:,+-@%=".contains(c) => { in_word = true; cur.push(c); }
            c if c.is_control() => return Err(Reject::Control),
            '\\' => return Err(Reject::Backslash),
            '$' | '`' | '~' | '*' | '?' | '[' | ']' | '{' | '}' | '#' | '!' => return Err(Reject::Expansion(c)),
            '|' | '&' | ';' | '<' | '>' | '(' | ')' => return Err(Reject::Operator(c)),
            c if !c.is_ascii() => return Err(Reject::NonAscii),
            c => return Err(Reject::Operator(c)),
        }
    }
    if in_word { argv.push(cur); }
    let first = argv.first().ok_or(Reject::Empty)?;
    if first.contains('=') { return Err(Reject::Assignment); }
    if first.starts_with('%') { return Err(Reject::Expansion('%')); }
    if KEYWORDS.contains(&first.as_str()) { return Err(Reject::Keyword(first.clone())); }
    // zsh `=cmd` expansion applies to any word starting with '='
    if argv.iter().any(|w| w.starts_with('=')) { return Err(Reject::Expansion('=')); }
    Ok(argv)
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let s: String = serde_json::from_str(&line).unwrap();
        let out = match mode.as_str() {
            "strict" => parse_strict(&s).ok(),
            _ => shlex::split(&s),
        };
        println!("{}", serde_json::to_string(&out).unwrap());
    }
}
