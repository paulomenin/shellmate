//! Property tests for the code that handles hostile input: the command classifier, the strict parser the
//! `exec` tool relies on, the reply parser, the streaming display filter and the terminal sanitizer.
//!
//! Example tests show that known attacks are caught. These check the rules that must hold for *every*
//! input, with an oracle written independently of the implementation where one is needed.

use proptest::prelude::*;
use shellmate_core::command::{classify, glob_match, parse_strict, rule_matches, Verdict};
use shellmate_core::response::{parse, DisplayFilter};
use shellmate_core::sanitize::{escape, is_buffer_safe};

/// Text full of the characters shells and terminals treat specially, plus arbitrary unicode.
fn nasty() -> impl Strategy<Value = String> {
    let atoms = prop_oneof![
        Just(" ".to_string()),
        Just("\t".to_string()),
        Just("\n".to_string()),
        Just("'".to_string()),
        Just("\"".to_string()),
        Just("\\".to_string()),
        Just("$(".to_string()),
        Just("`".to_string()),
        Just("${".to_string()),
        Just(";".to_string()),
        Just("&&".to_string()),
        Just("||".to_string()),
        Just("|".to_string()),
        Just(">".to_string()),
        Just("<<".to_string()),
        Just("~".to_string()),
        Just("*".to_string()),
        Just("=".to_string()),
        Just("\u{1b}[".to_string()),
        Just("\u{202e}".to_string()),
        Just("\0".to_string()),
        Just("rm".to_string()),
        Just("sudo".to_string()),
        Just("ls".to_string()),
        "[a-z]{1,6}",
        "\\PC{1,4}",
    ];
    prop::collection::vec(atoms, 0..14).prop_map(|v| v.concat())
}

/// A plain word the strict grammar must accept anywhere except as the first word of some keywords.
fn word() -> impl Strategy<Value = String> {
    "[A-Za-z][A-Za-z0-9_./:,+@%-]{0,10}"
}

/// Independent oracle: remove well-formed quoted spans, then report whether anything outside them,
/// or inside them, is something a simple command may not contain.
fn oracle_says_simple(s: &str) -> bool {
    let mut outside = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\'' || c == '"' {
            let mut closed = false;
            for d in chars.by_ref() {
                if d == c {
                    closed = true;
                    break;
                }
                if d.is_control() || d == '\\' || (c == '"' && matches!(d, '$' | '`' | '!')) {
                    return false;
                }
            }
            if !closed {
                return false;
            }
            outside.push('q');
        } else {
            outside.push(c);
        }
    }
    outside
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " \t_./:,+-@%=~#q".contains(c))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, failure_persistence: None, ..ProptestConfig::default() })]

    /// Nothing the model can say makes the classifier or the parsers panic.
    #[test]
    fn hostile_text_never_panics_the_parsers(s in nasty()) {
        let _ = parse_strict(&s);
        let _ = classify(&s);
        let _ = parse(&s);
        let _ = escape(&s);
    }

    /// If the strict parser accepts a line, the line contains no shell syntax at all: an independent
    /// oracle agrees. This is what lets `exec` run argv directly with no shell.
    #[test]
    fn accepted_lines_contain_no_shell_syntax(s in nasty()) {
        if parse_strict(&s).is_ok() {
            prop_assert!(oracle_says_simple(&s), "accepted but not simple: {s:?}");
        }
    }

    /// A line that is not a simple command is never `Safe`, and `Safe` implies a simple command.
    #[test]
    fn only_simple_commands_can_be_safe(s in nasty()) {
        let a = classify(&s);
        if parse_strict(&s).is_err() {
            prop_assert!(a.verdict == Verdict::Unsafe, "{:?}", s);
        }
        if a.verdict == Verdict::Safe {
            prop_assert!(parse_strict(&s).is_ok());
        }
    }

    /// Wrapping a destructive or privileged program in any amount of quoting, path prefix or arguments
    /// never makes it safe.
    #[test]
    fn destructive_programs_stay_unsafe_however_they_are_written(
        prog in prop::sample::select(vec!["rm", "sudo", "dd", "chmod", "kill", "mkfs", "doas", "shred"]),
        dir in prop::sample::select(vec!["", "/bin/", "/usr/bin/", "./", "../"]),
        quote in prop::sample::select(vec!["", "'", "\""]),
        args in prop::collection::vec(word(), 0..4),
    ) {
        let line = format!("{dir}{quote}{prog}{quote} {}", args.join(" "));
        let a = classify(&line);
        prop_assert!(a.verdict == Verdict::Unsafe, "{:?}", line);
    }

    /// Whatever precedes or follows, a destructive program after a shell operator or substitution is caught.
    #[test]
    fn destructive_programs_are_caught_after_any_operator(
        before in nasty(),
        op in prop::sample::select(vec![";", "&&", "||", "|", "\n", "$(", "`", "&", "(", "{ "]),
        prog in prop::sample::select(vec!["rm -rf /", "sudo ls", "dd if=/dev/zero of=x"]),
        after in nasty().prop_filter("a quote could close early", |s| !s.contains(['\'', '"'])),
    ) {
        let line = format!("{before}{op} {prog} {after}");
        prop_assert!(classify(&line).verdict == Verdict::Unsafe, "{:?}", line);
    }

    /// Plain words survive the parser unchanged, one argument per word.
    #[test]
    fn plain_words_round_trip(words in prop::collection::vec(word(), 1..6)) {
        let line = words.join(" ");
        // a plain first word that is a shell keyword is refused on purpose
        if let Ok(argv) = parse_strict(&line) {
            prop_assert_eq!(argv, words);
        }
    }

    /// A quoted word is one argument, whatever spaces it holds, and nothing in it is interpreted.
    #[test]
    fn single_quotes_keep_a_word_whole(inner in "[ A-Za-z0-9;|&<>$`*?~#!(){}\\[\\]-]{0,12}") {
        let line = format!("echo '{inner}'");
        let argv = parse_strict(&line).expect("single-quoted text is literal");
        prop_assert_eq!(argv, vec!["echo".to_string(), inner]);
    }

    /// Sensitive characters in double quotes are refused, never half-parsed.
    #[test]
    fn expansions_in_double_quotes_are_refused(
        pre in "[a-z ]{0,5}", c in prop::sample::select(vec!['$', '`', '!', '\\']), post in "[a-z ]{0,5}",
    ) {
        let line = format!("echo \"{pre}{c}{post}\"");
        prop_assert!(parse_strict(&line).is_err(), "{line:?}");
    }

    /// Rules: `*` as a program never approves anything, and a rule never matches a different program.
    #[test]
    fn rules_cannot_be_widened_by_wildcards(
        prog in word(), other in word(), args in prop::collection::vec(word(), 0..3),
    ) {
        let mut argv = vec![other.clone()];
        argv.extend(args);
        prop_assert!(!rule_matches("*", &argv));
        prop_assert!(!rule_matches("* *", &argv));
        prop_assert!(!rule_matches("?* *", &argv));
        if prog != other {
            let rule = format!("{prog} *");
            prop_assert!(!rule_matches(&rule, &argv));
        }
    }

    #[test]
    fn glob_matching_agrees_with_its_definition(p in "[a-c*?]{0,6}", t in "[a-c]{0,8}") {
        // the textbook recursive definition
        fn m(p: &[char], t: &[char]) -> bool {
            match p.first() {
                None => t.is_empty(),
                Some('*') => m(&p[1..], t) || (!t.is_empty() && m(p, &t[1..])),
                Some('?') => !t.is_empty() && m(&p[1..], &t[1..]),
                Some(c) => t.first() == Some(c) && m(&p[1..], &t[1..]),
            }
        }
        let (pc, tc): (Vec<char>, Vec<char>) = (p.chars().collect(), t.chars().collect());
        prop_assert_eq!(glob_match(&p, &t), m(&pc, &tc));
    }

    /// Escaped text is inert on a terminal: no control characters, and escaping twice changes nothing
    /// about what is visible (no raw escape survives).
    #[test]
    fn escaped_text_has_no_control_or_bidi_characters(s in nasty()) {
        let e = escape(&s);
        prop_assert!(!e.chars().any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c)), "{e:?}");
    }

    /// Everything the reply parser offers as a command is safe to put in the input buffer, and a
    /// command can never contain a line that starts a code fence (it would end its own block).
    #[test]
    fn suggested_commands_are_buffer_safe(s in nasty(), cmd in nasty()) {
        let text = format!("{s}\n```shellmate-command\n{cmd}\n```\n{s}");
        for c in parse(&text).commands {
            prop_assert!(c.chars().all(is_buffer_safe), "{c:?}");
            prop_assert!(c.lines().all(|l| !l.trim_start().starts_with("```")), "{c:?}");
        }
    }
}

/// A reply built from prose lines and command blocks whose bodies carry a marker the user must never see.
fn reply() -> impl Strategy<Value = String> {
    let prose = "[A-Za-z .,]{0,30}".prop_map(|l| format!("{l}\n"));
    let block = prop::sample::select(vec!["shellmate-command", "shellmate-tool"])
        .prop_map(|k| format!("```{k}\nLEAK-ls -la\n```\n"));
    prop::collection::vec(prop_oneof![prose, block], 0..8).prop_map(|v| v.concat())
}

fn stream(text: &str, cuts: &[usize]) -> String {
    let mut f = DisplayFilter::default();
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    for &cut in cuts {
        let end = (at + cut).min(chars.len());
        out.push_str(&f.push(&chars[at..end].iter().collect::<String>()));
        at = end;
    }
    out.push_str(&f.push(&chars[at..].iter().collect::<String>()));
    out.push_str(&f.finish());
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1000, failure_persistence: None, ..ProptestConfig::default() })]

    /// However the network splits the stream, the user sees the same text, and never the body of a
    /// command or tool block.
    #[test]
    fn streaming_display_is_independent_of_chunking_and_hides_blocks(
        text in reply(), cuts in prop::collection::vec(1usize..9, 0..20),
    ) {
        let whole = stream(&text, &[]);
        let chunked = stream(&text, &cuts);
        prop_assert_eq!(&whole, &chunked);
        prop_assert!(!whole.contains("LEAK"), "{whole:?}");
    }

    /// The streamed text and the final parse agree on what is a command: no marker in the prose either.
    #[test]
    fn parsed_prose_never_holds_a_block_body(text in reply()) {
        let a = parse(&text);
        prop_assert!(!a.prose.contains("LEAK"), "{:?}", a.prose);
    }
}
