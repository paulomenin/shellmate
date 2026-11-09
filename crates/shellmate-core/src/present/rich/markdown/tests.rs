use super::*;
use crate::present::{Level, Theme};

fn run(text: &str, width: usize, level: Level, unicode: bool, chunk: usize) -> String {
    let mut wrap = Wrap::new(width, Theme::new(level));
    let mut md = Markdown::new(width, unicode);
    let chars: Vec<char> = text.chars().collect();
    for piece in chars.chunks(chunk.max(1)) {
        md.push(&piece.iter().collect::<String>(), &mut wrap);
    }
    md.finish(&mut wrap);
    wrap.take()
}

/// The structure of `text` as plain text: no styles, Unicode symbols, 40 columns.
fn plain(text: &str) -> String {
    run(text, 40, Level::Plain, true, usize::MAX)
}

fn ansi(text: &str) -> String {
    run(text, 40, Level::Ansi16, true, usize::MAX)
}

// ---- inline spans ----

#[test]
fn the_markers_of_strong_emphasis_and_code_are_not_shown() {
    assert_eq!(plain("a **b** c"), "a b c");
    assert_eq!(plain("a *b* c"), "a b c");
    assert_eq!(plain("a _b_ c"), "a b c");
    assert_eq!(plain("a `b` c"), "a b c");
    assert_eq!(plain("a __b__ c"), "a b c");
}

#[test]
fn each_span_gets_its_own_style() {
    assert!(ansi("**b**").contains("\u{1b}[1mb\u{1b}[0m"));
    assert!(ansi("*b*").contains("\u{1b}[3mb\u{1b}[0m"));
    assert!(
        ansi("`b`").contains("\u{1b}[35mb\u{1b}[0m"),
        "{:?}",
        ansi("`b`")
    );
    let both = ansi("***b***");
    assert!(
        both.contains("\u{1b}[1m") && both.contains("\u{1b}[3m"),
        "{both:?}"
    );
}

#[test]
fn a_span_that_never_closes_is_shown_as_written() {
    assert_eq!(plain("a **b c"), "a **b c");
    assert_eq!(plain("a `b c"), "a `b c");
    assert_eq!(plain("*.txt and *.log files"), "*.txt and *.log files");
    assert_eq!(plain("**a\nb**"), "**a\nb**");
}

#[test]
fn stars_and_underscores_that_are_not_emphasis_are_left_alone() {
    assert_eq!(plain("2 * 3 * 4"), "2 * 3 * 4");
    assert_eq!(plain("snake_case_name here"), "snake_case_name here");
    assert_eq!(plain("a * b"), "a * b");
    assert_eq!(plain("rm -rf *"), "rm -rf *");
    assert_eq!(plain("trailing *"), "trailing *");
    assert_eq!(plain("a____b"), "a____b");
}

#[test]
fn spans_can_nest_and_code_is_always_code() {
    let out = ansi("**use `ls` now**");
    assert!(out.contains("\u{1b}[1muse\u{1b}[0m"), "{out:?}");
    assert!(out.contains("\u{1b}[35mls\u{1b}[0m"), "{out:?}");
    assert_eq!(plain("**use `ls` now**"), "use ls now");
    assert_eq!(plain("*a **b** c*"), "a b c");
}

#[test]
fn delimiters_inside_code_are_text() {
    assert_eq!(plain("`a*b*c` and `x_y_z`"), "a*b*c and x_y_z");
    assert_eq!(plain("``a`b``"), "a`b");
    assert_eq!(plain("`a``"), "`a``");
}

#[test]
fn a_heading_keeps_its_own_style_around_strong_text() {
    let out = ansi("# a **b** c");
    assert!(
        !out.contains("\u{1b}[1mb"),
        "strong inside a heading changes nothing: {out:?}"
    );
    assert_eq!(plain("# a **b** c"), "a b c");
}

// ---- blocks ----

#[test]
fn headings_lose_their_hashes_and_get_a_style() {
    assert_eq!(plain("# Title\ntext"), "Title\ntext");
    assert_eq!(plain("### Three"), "Three");
    assert!(
        ansi("# T").contains("\u{1b}[1m\u{1b}[36mT") || ansi("# T").contains("36"),
        "{:?}",
        ansi("# T")
    );
}

#[test]
fn a_hash_that_is_not_a_heading_is_text() {
    assert_eq!(plain("#hashtag"), "#hashtag");
    assert_eq!(plain("####### seven"), "####### seven");
    assert_eq!(plain("#"), "#");
}

#[test]
fn bullets_get_one_marker_whatever_they_were_written_with() {
    assert_eq!(plain("- one\n- two"), "• one\n• two");
    assert_eq!(plain("* one\n+ two"), "• one\n• two");
    assert_eq!(run("- one", 40, Level::Plain, false, usize::MAX), "- one");
}

#[test]
fn numbered_items_keep_their_numbers() {
    assert_eq!(plain("1. one\n2. two"), "1. one\n2. two");
    assert_eq!(plain("10) ten"), "10) ten");
}

#[test]
fn a_number_that_is_not_a_list_marker_is_text() {
    assert_eq!(plain("3 apples"), "3 apples");
    assert_eq!(plain("1.5 liters"), "1.5 liters");
    assert_eq!(plain("-5 degrees"), "-5 degrees");
    assert_eq!(plain("12"), "12");
}

#[test]
fn nested_items_are_indented_and_wrapped_lines_hang_under_their_text() {
    assert_eq!(plain("- a\n  - b"), "• a\n  • b");
    let out = run(
        "- alpha beta gamma delta",
        14,
        Level::Plain,
        true,
        usize::MAX,
    );
    assert_eq!(out, "• alpha beta\n  gamma delta");
}

#[test]
fn a_line_after_an_item_continues_it_and_a_blank_line_ends_it() {
    assert_eq!(plain("- a\nmore\n\nafter"), "• a\n  more\n\nafter");
}

#[test]
fn quotes_get_a_bar_on_every_line() {
    assert_eq!(plain("> hi\n> there"), "▎ hi\n▎ there");
    assert_eq!(run("> hi", 40, Level::Plain, false, usize::MAX), "| hi");
}

#[test]
fn a_rule_is_a_line_and_strong_text_at_the_start_of_a_line_is_not_a_rule() {
    let line = format!("{}\n", "─".repeat(40));
    assert_eq!(plain("---"), line, "at the end of the text");
    assert_eq!(plain("---\n"), line);
    assert_eq!(plain("***\n"), line);
    assert_eq!(plain("___\n"), line);
    assert_eq!(
        plain("- - -\n"),
        "• - -\n",
        "a bullet, as in CommonMark's reading of it"
    );
    let ascii = run("---", 20, Level::Plain, false, usize::MAX);
    assert_eq!(ascii, format!("{}\n", "-".repeat(20)));
    assert_eq!(plain("--"), "--");
    assert_eq!(plain("**bold** start"), "bold start");
}

#[test]
fn a_fenced_block_is_shown_as_it_is_with_a_bar() {
    assert_eq!(
        plain("```sh\nls -la\n  x\n```\nafter"),
        "│ ls -la\n│   x\nafter"
    );
    assert_eq!(plain("```\nplain\n```"), "│ plain\n");
    assert_eq!(plain("~~~\nplain\n~~~"), "│ plain\n");
}

#[test]
fn nothing_is_formatted_inside_a_fenced_block() {
    assert_eq!(
        plain("```\n**x** # y - z `w`\n```"),
        "│ **x** # y - z `w`\n"
    );
}

#[test]
fn a_fence_closes_only_with_enough_marks_and_nothing_else_on_the_line() {
    assert_eq!(plain("````\na\n```\nb\n````"), "│ a\n│ ```\n│ b\n");
    assert_eq!(plain("```\n```x\n```"), "│ ```x\n");
}

#[test]
fn an_unclosed_block_ends_with_the_text() {
    assert_eq!(plain("```\nunfinished"), "│ unfinished");
    assert_eq!(plain("```\na\n```"), "│ a\n");
}

#[test]
fn a_marker_with_nothing_after_it_is_still_shown() {
    assert_eq!(plain("1. \nnext"), "1. \n   next");
    assert_eq!(plain("- \n"), "• \n");
    assert_eq!(plain("1.  "), "1. ");
    assert_eq!(plain("> \n> x"), "▎ \n▎ x");
}

#[test]
fn a_table_row_is_shown_as_written() {
    assert_eq!(plain("| a | b |"), "| a | b |");
}

#[test]
fn blank_lines_between_paragraphs_are_kept() {
    assert_eq!(plain("one\n\ntwo"), "one\n\ntwo");
}

#[test]
fn long_text_wraps_at_the_width() {
    let out = run("alpha beta gamma delta", 12, Level::Plain, true, usize::MAX);
    assert_eq!(out, "alpha beta\ngamma delta");
}

// ---- safety ----

#[test]
fn escape_sequences_are_shown_never_obeyed_in_every_kind_of_block() {
    for text in [
        "x \u{1b}[2J y",
        "# h \u{1b}[2J",
        "- i \u{1b}[2J",
        "> q \u{1b}[2J",
        "```\nc \u{1b}[2J\n```",
        "**b \u{1b}[2J**",
        "`c \u{1b}]0;title\u{7}`",
    ] {
        for level in [Level::Plain, Level::Mono, Level::Ansi16] {
            let out = run(text, 40, level, true, usize::MAX);
            let stripped = strip_sgr(&out);
            assert!(
                !stripped.contains('\u{1b}') && !stripped.contains('\u{7}'),
                "{text:?} {out:?}"
            );
        }
    }
}

/// `s` without its `ESC [ ... m` sequences.
fn strip_sgr(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for d in chars.by_ref() {
                if d == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn the_styled_text_is_the_unstyled_text() {
    let doc = "# T\n\nSome **bold** and `code`.\n\n- a\n- b *c*\n\n> q\n\n```\nx\n```\n";
    assert_eq!(
        strip_sgr(&run(doc, 30, Level::Ansi16, true, usize::MAX)),
        run(doc, 30, Level::Plain, true, usize::MAX)
    );
}

// ---- chunking and limits ----

const DOC: &str = "# Title here\n\nSome **bold**, *it* and `code` text that is long enough to wrap around the width.\n\n- one **a**\n  - two `b`\n1. num\n\n> quote *x*\n\n---\n\n```sh\nls  -la\n```\n\n2 * 3 * 4 and *.txt and _snake_case_ and __init__.\n**unclosed and `code";

#[test]
fn the_output_is_the_same_however_the_text_is_split() {
    for level in [Level::Plain, Level::Mono, Level::Ansi16] {
        let whole = run(DOC, 30, level, true, usize::MAX);
        for chunk in [1, 2, 3, 5, 7, 11] {
            assert_eq!(
                run(DOC, 30, level, true, chunk),
                whole,
                "{level:?} chunk {chunk}"
            );
        }
    }
}

#[test]
fn an_endless_unclosed_span_is_shown_before_the_end_of_the_line() {
    let text = format!("*{}", "xx ".repeat(MAX_HELD / 3 + 10));
    let mut wrap = Wrap::new(80, Theme::new(Level::Plain));
    let mut md = Markdown::new(80, true);
    md.push(&text, &mut wrap);
    let out = wrap.take();
    assert!(
        out.starts_with("*xx xx xx"),
        "held text must be released at the limit"
    );
}

#[test]
fn a_notice_interrupts_the_line_being_read() {
    let mut wrap = Wrap::new(40, Theme::new(Level::Plain));
    let mut md = Markdown::new(40, true);
    md.push("half a **line", &mut wrap);
    md.interrupt(&mut wrap);
    md.push("next\n", &mut wrap);
    md.finish(&mut wrap);
    assert_eq!(wrap.take(), "half a **line\nnext\n");
}

#[test]
fn interrupting_with_nothing_pending_writes_nothing() {
    let mut wrap = Wrap::new(40, Theme::new(Level::Plain));
    let mut md = Markdown::new(40, true);
    md.interrupt(&mut wrap);
    assert_eq!(wrap.take(), "");
    md.push("a\n", &mut wrap);
    md.interrupt(&mut wrap);
    md.finish(&mut wrap);
    assert_eq!(wrap.take(), "a\n");
}

// ---- block kinds at the edges ----

#[test]
fn a_heading_has_one_to_six_hashes() {
    assert_eq!(plain("###### six"), "six");
    assert_eq!(plain("####### seven"), "####### seven");
}

#[test]
fn an_underscore_and_a_space_is_not_a_list_item() {
    assert_eq!(plain("_ item"), "_ item");
    assert_eq!(plain("- item"), "\u{2022} item");
}

#[test]
fn rules_may_have_spaces_between_the_marks() {
    let rule = "\u{2500}".repeat(40) + "\n";
    for text in ["-- -", "** *", "_ _ _"] {
        assert_eq!(plain(text), rule, "{text:?}");
    }
    assert_eq!(plain("-- x"), "-- x");
    assert_eq!(plain("__ x"), "__ x");
}

#[test]
fn a_fence_with_no_language_opens_a_code_block() {
    assert_eq!(plain("```\ncode\n```"), "\u{2502} code\n");
    assert_eq!(plain("~~~\nx\n~~~"), "\u{2502} x\n");
    assert_eq!(plain("  ```\nx\n  ```"), "\u{2502} x\n");
}

#[test]
fn only_a_matching_fence_closes_a_code_block() {
    // a longer text after the marks, or fewer marks, does not close it
    assert_eq!(plain("```\nx\n``` y\n```"), "\u{2502} x\n\u{2502} ``` y\n");
    assert_eq!(plain("```\nx\n ``\n```"), "\u{2502} x\n\u{2502}  ``\n");
    // the other kind of mark does not close it either
    assert_eq!(plain("```\nx\n~~~\n```"), "\u{2502} x\n\u{2502} ~~~\n");
    assert_eq!(plain("~~~\nx\n```\n~~~"), "\u{2502} x\n\u{2502} ```\n");
    // up to three spaces of indent still close it, four do not
    assert_eq!(plain("```\nx\n   ```\ny"), "\u{2502} x\ny");
    assert_eq!(
        plain("```\nx\n    ```\ny"),
        "\u{2502} x\n\u{2502}     ```\n\u{2502} y"
    );
}

#[test]
fn emphasis_next_to_words() {
    assert_eq!(plain("snake_case_name"), "snake_case_name");
    assert_eq!(plain("a **b*"), "a **b*");
    assert_eq!(plain("**a*b**"), "**a*b**");
    assert_eq!(plain("`a` `b`"), "a b");
    assert_eq!(plain("**a** **b**"), "a b");
    assert_eq!(plain("_a_b"), "_a_b");
    assert_eq!(plain("a_b_ c"), "a_b_ c");
    assert_eq!(plain("a ** b**"), "a ** b**");
    assert_eq!(plain("**a **"), "**a **");
}

fn feed(chunks: &[&str]) -> String {
    let mut wrap = Wrap::new(40, Theme::new(Level::Plain));
    let mut md = Markdown::new(40, true);
    for c in chunks {
        md.push(c, &mut wrap);
    }
    wrap.take()
}

#[test]
fn code_is_shown_as_soon_as_it_cannot_be_a_closing_fence() {
    assert_eq!(feed(&["```\n", "hel"]), "\u{2502} hel");
    assert_eq!(feed(&["```\n", "  hel"]), "\u{2502}   hel");
    assert_eq!(feed(&["```\n", "`` x"]), "\u{2502} `` x");
    assert_eq!(feed(&["```\n", "    `"]), "\u{2502}     `");
    // these might still become a closing fence
    assert_eq!(feed(&["```\n", "`` "]), "");
    assert_eq!(feed(&["```\n", "   ``"]), "");
}

#[test]
fn a_closing_fence_may_have_trailing_spaces() {
    assert_eq!(plain("```\nx\n``` \ny"), "\u{2502} x\ny");
    assert_eq!(plain("```\nx\n```  \ny"), "\u{2502} x\ny");
}

#[test]
fn only_a_short_number_makes_an_ordered_list() {
    assert!(ansi("9. word").contains("\u{1b}[36m9. "));
    assert!(!ansi("1234567890. word").contains('\u{1b}'));
    let long = "1234567890. word word word word word word word word word";
    assert!(!plain(long).contains("\n "), "{:?}", plain(long));
}

#[test]
fn different_spans_next_to_each_other_keep_their_own_style() {
    assert_eq!(ansi("**a**`b`"), "\u{1b}[1ma\u{1b}[0m\u{1b}[35mb\u{1b}[0m");
    assert_eq!(ansi("*_a_*"), "\u{1b}[3ma\u{1b}[0m");
}

#[test]
fn a_span_may_hold_up_to_the_limit_and_no_more() {
    let body = |n: usize| "a".repeat(n);
    let at = format!("**{}**", body(MAX_HELD));
    let over = format!("**{}**", body(MAX_HELD + 1));
    assert_eq!(plain(&at).replace('\n', "").len(), MAX_HELD);
    assert_eq!(plain(&over).replace('\n', "").len(), MAX_HELD + 5);
}

#[test]
fn only_backticks_and_tildes_make_a_fence() {
    assert_eq!(plain("aaa\nb"), "aaa\nb");
    assert_eq!(plain("``\nb"), "``\nb");
    assert_eq!(plain("---x\nb"), "---x\nb");
}
