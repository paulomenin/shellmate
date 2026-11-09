use super::*;
use crate::present::Level;
use crate::render::wrap as old_wrap;

fn plain(width: usize) -> Wrap {
    Wrap::new(width, Theme::new(Level::Plain))
}

fn colour(width: usize) -> Wrap {
    Wrap::new(width, Theme::new(Level::Ansi16))
}

fn render(mut w: Wrap, f: impl FnOnce(&mut Wrap)) -> String {
    f(&mut w);
    w.finish();
    w.take()
}

fn words(width: usize, text: &str) -> String {
    render(plain(width), |w| w.text(text, Role::Plain))
}

#[test]
fn text_without_styles_wraps_exactly_like_the_plain_wrapper() {
    for (text, width) in [
        ("aaa bbb ccc ddd", 10),
        ("one\n\ntwo", 20),
        ("abcdefghijklmnopqrstuvwxyz", 10),
        ("aaaa bbbbb", 10),
        ("aaaa bbbbbc", 10),
        ("   leading and   inner   spaces", 12),
        ("hi \u{1b}[2J there", 40),
        ("aaaa \u{1}\u{1}", 10),
        ("To list the largest files, use du with a very_long_identifier_that_exceeds_the_width.\nNext", 24),
    ] {
        assert_eq!(words(width, text), old_wrap(text, width), "{text:?} at {width}");
    }
}

#[test]
fn nothing_is_written_until_a_word_is_complete() {
    let mut w = plain(40);
    w.text("Hel", Role::Plain);
    assert_eq!(w.take(), "");
    w.text("lo wor", Role::Plain);
    assert_eq!(w.take(), "Hello");
    w.finish();
    assert_eq!(w.take(), " wor");
}

#[test]
fn the_output_does_not_depend_on_how_the_text_is_split() {
    let text =
        "Some **text** with `code`, a verylongwordthatneedstobecutbythewrapper and\n\nparagraphs.";
    let one = render(colour(20), |w| w.text(text, Role::Strong));
    let by_char = render(colour(20), |w| {
        for c in text.chars() {
            w.text(&c.to_string(), Role::Strong);
        }
    });
    assert_eq!(one, by_char);
}

#[test]
fn a_styled_word_is_painted_and_reset_and_the_space_is_not() {
    let out = render(colour(40), |w| {
        w.text("see ", Role::Plain);
        w.text("this", Role::Strong);
        w.text(" now", Role::Plain);
    });
    assert!(out.contains("\u{1b}[1mthis\u{1b}[0m"), "{out:?}");
    assert!(out.starts_with("see ") && out.ends_with(" now"), "{out:?}");
}

#[test]
fn a_word_made_of_two_styles_is_painted_in_pieces() {
    let out = render(colour(40), |w| {
        w.text("ab", Role::Strong);
        w.text("cd", Role::Plain);
    });
    assert_eq!(out, "\u{1b}[1mab\u{1b}[0mcd");
}

#[test]
fn escape_sequences_in_the_text_are_shown_never_obeyed() {
    let out = render(colour(60), |w| {
        w.text("x \u{1b}[31mred\u{1b}[0m \u{7}", Role::Plain)
    });
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(out.contains("\\u{1b}[31mred"), "{out:?}");
}

#[test]
fn width_counts_visible_columns_not_escape_sequences() {
    // each styled word is 4 columns wide but 13 bytes long; three of them fit in 14 columns
    let out = render(colour(14), |w| {
        for i in 0..3 {
            if i > 0 {
                w.text(" ", Role::Plain);
            }
            w.text("word", Role::Error);
        }
    });
    assert_eq!(out.matches('\n').count(), 0, "{out:?}");
    let narrow = render(colour(13), |w| {
        for i in 0..3 {
            if i > 0 {
                w.text(" ", Role::Plain);
            }
            w.text("word", Role::Error);
        }
    });
    assert_eq!(narrow.matches('\n').count(), 1, "{narrow:?}");
}

#[test]
fn a_block_prefix_starts_the_first_line_and_a_hanging_one_the_rest() {
    let out = render(plain(14), |w| {
        w.block(("- ", Role::Marker), ("  ", Role::Plain));
        w.text("alpha beta gamma delta", Role::Plain);
    });
    assert_eq!(out, "- alpha beta\n  gamma delta");
}

#[test]
fn a_new_block_starts_with_its_first_prefix_again() {
    let out = render(plain(40), |w| {
        w.block(("1. ", Role::Marker), ("   ", Role::Plain));
        w.text("one", Role::Plain);
        w.newline();
        w.block(("2. ", Role::Marker), ("   ", Role::Plain));
        w.text("two", Role::Plain);
    });
    assert_eq!(out, "1. one\n2. two");
}

#[test]
fn an_empty_line_has_no_prefix_and_ending_the_block_removes_it() {
    let out = render(plain(40), |w| {
        w.block(("> ", Role::Quote), ("> ", Role::Quote));
        w.text("a", Role::Plain);
        w.newline();
        w.newline();
        w.text("b", Role::Plain);
        w.newline();
        w.end_block();
        w.text("c", Role::Plain);
    });
    assert_eq!(out, "> a\n\n> b\nc");
}

#[test]
fn a_word_longer_than_the_room_is_cut_and_the_prefix_repeats() {
    let out = render(plain(10), |w| {
        w.block(("- ", Role::Marker), ("  ", Role::Plain));
        w.text("abcdefghijklmnopqrst", Role::Plain);
    });
    assert_eq!(out, "- abcdefgh\n  ijklmnop\n  qrst");
    assert!(out.lines().all(|l| l.chars().count() <= 10));
}

#[test]
fn a_prefix_that_would_take_half_the_line_is_dropped() {
    let out = render(plain(20), |w| {
        w.block(
            (&" ".repeat(12), Role::Plain),
            (&" ".repeat(12), Role::Plain),
        );
        w.text("text", Role::Plain);
    });
    assert_eq!(out, "text");
}

#[test]
fn preformatted_text_keeps_its_spaces_and_is_cut_not_re_flowed() {
    let out = render(plain(10), |w| {
        w.block(("| ", Role::Muted), ("| ", Role::Muted));
        w.raw("  indented\n", Role::CodeBlock);
        w.raw("0123456789abcdef\n", Role::CodeBlock);
        w.raw("a\tb", Role::CodeBlock);
    });
    assert_eq!(out, "|   indent\n| ed\n| 01234567\n| 89abcdef\n| a    b");
}

#[test]
fn preformatted_text_is_escaped_too() {
    let out = render(plain(40), |w| w.raw("echo \u{1b}[2J\n", Role::CodeBlock));
    assert!(
        !out.contains('\u{1b}') && out.contains("\\u{1b}[2J"),
        "{out:?}"
    );
}

#[test]
fn preformatted_text_is_the_same_however_it_is_split() {
    let text =
        "  fn main() {\n      println!(\"hello world, a long line that must be cut\");\n  }\n";
    let one = render(colour(16), |w| w.raw(text, Role::Code));
    let by_char = render(colour(16), |w| {
        for c in text.chars() {
            w.raw(&c.to_string(), Role::Code);
        }
    });
    assert_eq!(one, by_char);
}

#[test]
fn a_blank_line_between_blocks_is_kept() {
    assert_eq!(words(40, "a\n\nb"), "a\n\nb");
}

#[test]
fn a_word_may_end_exactly_at_the_width() {
    assert_eq!(words(6, "abc de"), "abc de");
    assert_eq!(words(5, "ab cd"), "ab cd");
}

#[test]
fn a_prefix_of_half_the_width_is_kept_and_a_longer_one_is_dropped() {
    let at = render(plain(10), |w| {
        w.block(("12345", Role::Plain), ("     ", Role::Plain));
        w.text("x", Role::Plain);
    });
    assert_eq!(at, "12345x");
    let over = render(plain(10), |w| {
        w.block(("123456", Role::Plain), ("      ", Role::Plain));
        w.text("x", Role::Plain);
    });
    assert_eq!(over, "x");
}
