//! Property tests for what reaches the terminal: for any text a model can write, the rich presenter writes
//! only its own styles, shows the same text at every level, never depends on how the text arrived, never
//! overflows a line, and never loses or invents a letter or a digit.

use proptest::prelude::*;
use shellmate_core::present::{Caps, Level, PlainPresenter, Presenter, RichPresenter};

fn caps(level: Level, markdown: bool, width: usize) -> Caps {
    Caps {
        width,
        unicode: true,
        level,
        markdown,
    }
}

/// Markdown punctuation, escape sequences, control characters and arbitrary unicode, mixed with words.
fn model_text() -> impl Strategy<Value = String> {
    let atoms = prop_oneof![
        Just(" ".to_string()),
        Just("  ".to_string()),
        Just("\n".to_string()),
        Just("\n\n".to_string()),
        Just("\t".to_string()),
        Just("*".to_string()),
        Just("**".to_string()),
        Just("***".to_string()),
        Just("_".to_string()),
        Just("__".to_string()),
        Just("`".to_string()),
        Just("``".to_string()),
        Just("```".to_string()),
        Just("~~~".to_string()),
        Just("# ".to_string()),
        Just("## ".to_string()),
        Just("- ".to_string()),
        Just("+ ".to_string()),
        Just("> ".to_string()),
        Just("1. ".to_string()),
        Just("---".to_string()),
        Just("|".to_string()),
        Just("\\".to_string()),
        Just("\u{1b}[31m".to_string()),
        Just("\u{1b}[2J".to_string()),
        Just("\u{1b}]0;x\u{7}".to_string()),
        Just("\u{1b}".to_string()),
        Just("\r".to_string()),
        Just("\u{202e}".to_string()),
        Just("\0".to_string()),
        "[a-z]{1,8}",
        "[0-9]{1,3}",
        "\\PC{1,3}",
    ];
    prop::collection::vec(atoms, 0..40).prop_map(|v| v.concat())
}

/// Words, digits and markdown punctuation only: no escapes, no code fences.
fn benign_text() -> impl Strategy<Value = String> {
    let atoms = prop_oneof![
        Just(" ".to_string()),
        Just("\n".to_string()),
        Just("\n\n".to_string()),
        Just("*".to_string()),
        Just("**".to_string()),
        Just("_".to_string()),
        Just("`".to_string()),
        Just("# ".to_string()),
        Just("- ".to_string()),
        Just("> ".to_string()),
        Just("1. ".to_string()),
        Just("---".to_string()),
        Just("(".to_string()),
        Just(".".to_string()),
        "[a-z]{1,6}",
        "[0-9]{1,3}",
    ];
    prop::collection::vec(atoms, 0..40)
        .prop_map(|v| v.concat())
        .prop_filter("no code fences", |s| {
            !s.contains("```") && !s.contains("~~~")
        })
}

/// Feed `text` in pieces of `chunk` characters, then finish.
fn present(p: &mut dyn Presenter, text: &str, chunk: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for piece in chars.chunks(chunk.max(1)) {
        out += &p.answer(&piece.iter().collect::<String>());
    }
    out += &p.finish();
    out
}

fn rich(level: Level, markdown: bool, width: usize, text: &str, chunk: usize) -> String {
    present(
        &mut RichPresenter::new(caps(level, markdown, width)),
        text,
        chunk,
    )
}

/// `s` without its styles (`ESC [ digits and ; m`).
fn strip(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\u{1b}' && b.get(i + 1) == Some(&'[') {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == ';') {
                j += 1;
            }
            if b.get(j) == Some(&'m') {
                i = j + 1;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// Every escape is one of our styles or the line erase, and no other control character appears.
fn only_our_escapes(s: &str) -> bool {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            '\u{1b}' => {
                if i > 0
                    && b[i - 1] == '\r'
                    && b.get(i + 1) == Some(&'[')
                    && b.get(i + 2) == Some(&'K')
                {
                    i += 3;
                    continue;
                }
                if b.get(i + 1) != Some(&'[') {
                    return false;
                }
                let mut j = i + 2;
                while j < b.len() && (b[j].is_ascii_digit() || b[j] == ';') {
                    j += 1;
                }
                if b.get(j) != Some(&'m') {
                    return false;
                }
                i = j + 1;
                continue;
            }
            '\r' => {
                if !(b.get(i + 1) == Some(&'\u{1b}')
                    && b.get(i + 2) == Some(&'[')
                    && b.get(i + 3) == Some(&'K'))
                {
                    return false;
                }
            }
            '\n' => {}
            c if c.is_control() => return false,
            '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}' => return false,
            _ => {}
        }
        i += 1;
    }
    true
}

fn alphanumerics(s: &str) -> Vec<char> {
    let mut v: Vec<char> = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    v.sort_unstable();
    v
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1500, failure_persistence: None, ..ProptestConfig::default() })]

    /// Whatever the model writes, the only escapes that reach the terminal are the theme's own.
    #[test]
    fn nothing_the_model_writes_adds_an_escape(text in model_text(), chunk in 1usize..9) {
        for level in [Level::Mono, Level::Ansi16] {
            for markdown in [true, false] {
                let out = rich(level, markdown, 30, &text, chunk);
                prop_assert!(only_our_escapes(&out), "{level:?} markdown={markdown} {text:?} -> {out:?}");
            }
        }
    }

    /// The output is the same however the text is split into pieces.
    #[test]
    fn the_split_of_the_text_does_not_matter(text in model_text(), chunk in 1usize..12) {
        for level in [Level::Plain, Level::Mono, Level::Ansi16] {
            let whole = rich(level, true, 30, &text, usize::MAX);
            let split = rich(level, true, 30, &text, chunk);
            prop_assert_eq!(&whole, &split, "{:?} chunk {}", &text, chunk);
        }
    }

    /// Colour and styling only change how text looks: the words are the same at every level.
    #[test]
    fn every_level_shows_the_same_text(text in model_text()) {
        let plain_text = rich(Level::Plain, true, 30, &text, usize::MAX);
        let body = plain_text.strip_prefix("\r\u{1b}[K").unwrap_or(&plain_text);
        prop_assert!(!body.contains('\u{1b}'), "the plain level writes no escapes: {:?}", &plain_text);
        let mono = strip(&rich(Level::Mono, true, 30, &text, usize::MAX));
        let colour = strip(&rich(Level::Ansi16, true, 30, &text, usize::MAX));
        prop_assert_eq!(&mono, &plain_text, "{:?}", &text);
        prop_assert_eq!(&colour, &plain_text, "{:?}", &text);
    }

    /// With markdown off, the rich presenter lays text out exactly as the plain presenter does.
    #[test]
    fn without_markdown_the_layout_is_the_plain_layout(text in model_text(), width in 10usize..70, chunk in 1usize..9) {
        let plain = present(&mut PlainPresenter::new(Caps::plain(width, true)), &text, chunk);
        let styled = rich(Level::Plain, false, width, &text, chunk);
        prop_assert_eq!(styled, plain, "{:?}", &text);
    }

    /// No line is wider than the terminal.
    #[test]
    fn no_line_overflows_the_width(text in model_text(), width in 10usize..60) {
        let out = strip(&rich(Level::Ansi16, true, width, &text, usize::MAX));
        let out = out.strip_prefix("\r\u{1b}[K").unwrap_or(&out);
        for line in out.lines() {
            prop_assert!(line.chars().count() <= width, "{} > {width}: {line:?} from {:?}", line.chars().count(), &text);
        }
    }

    /// Formatting removes markers and nothing else: no letter or digit is lost or invented.
    #[test]
    fn no_letter_or_digit_is_lost_or_invented(text in benign_text()) {
        let out = strip(&rich(Level::Ansi16, true, 30, &text, usize::MAX));
        let out = out.strip_prefix("\r\u{1b}[K").unwrap_or(&out).to_string();
        prop_assert_eq!(alphanumerics(&out), alphanumerics(&text), "{:?} -> {:?}", &text, &out);
    }

    /// Output always ends on a fresh line, and finishing twice writes nothing more.
    #[test]
    fn the_output_ends_on_a_fresh_line(text in model_text()) {
        let mut p = RichPresenter::new(caps(Level::Ansi16, true, 30));
        let out = present(&mut p, &text, usize::MAX);
        prop_assert!(out.is_empty() || out.ends_with('\n'), "{out:?}");
        prop_assert_eq!(p.finish(), "");
    }
}
