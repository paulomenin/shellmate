use super::*;
use crate::present::{Level, PlainPresenter};

fn caps(level: Level, markdown: bool) -> Caps {
    Caps {
        width: 40,
        unicode: true,
        level,
        markdown,
    }
}

fn rich(level: Level, markdown: bool) -> RichPresenter {
    RichPresenter::new(caps(level, markdown))
}

/// `s` without its styles (`ESC [ digits and ; m`). The line erase is not a style and stays.
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

/// Every escape in `s` is a style (`ESC [ digits and ; m`) or the line erase after a carriage return.
fn only_styles(s: &str) -> bool {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\u{1b}' {
            if i > 0 && b[i - 1] == '\r' && b.get(i + 1) == Some(&'[') && b.get(i + 2) == Some(&'K')
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
        if b[i].is_control() && !matches!(b[i], '\n' | '\r') {
            return false;
        }
        i += 1;
    }
    true
}

/// The numeric parameters of every style in `s`.
fn sgr_params(s: &str) -> Vec<u32> {
    let b: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '\u{1b}' && b.get(i + 1) == Some(&'[') {
            let mut j = i + 2;
            let mut params = String::new();
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == ';') {
                params.push(b[j]);
                j += 1;
            }
            if b.get(j) == Some(&'m') {
                out.extend(params.split(';').filter_map(|n| n.parse::<u32>().ok()));
            }
            i = j;
        }
        i += 1;
    }
    out
}

fn notice() -> Notice {
    Notice::new(Tone::Success, Mark::Check, "ran (approved): ls")
        .with_meta("exit 0, 0.0s")
        .with_detail(vec!["one".into(), "two".into()])
}

#[test]
fn an_answer_is_formatted_and_the_first_output_erases_the_waiting_indicator() {
    let mut p = rich(Level::Ansi16, true);
    let out = p.answer("Some **bold** text\n");
    assert!(out.starts_with("\r\u{1b}[K"), "{out:?}");
    assert!(out.contains("\u{1b}[1mbold\u{1b}[0m"), "{out:?}");
    assert_eq!(strip(&out), "\r\u{1b}[KSome bold text\n");
    assert!(p.wrote_any());
    assert!(
        !p.answer("more ").contains('\r'),
        "only the first output erases"
    );
}

#[test]
fn with_markdown_off_the_answer_is_wrapped_text_as_written() {
    let mut p = rich(Level::Ansi16, false);
    let mut out = p.answer("Some **bold** text\n");
    out += &p.finish();
    assert_eq!(strip(&out), "\r\u{1b}[KSome **bold** text\n");
    assert!(!out.contains("\u{1b}[1m"));
}

#[test]
fn finish_ends_on_a_fresh_line_once() {
    let mut p = rich(Level::Mono, true);
    p.answer("one two");
    let end = p.finish();
    assert!(end.ends_with('\n'), "{end:?}");
    assert_eq!(p.finish(), "");
    assert_eq!(rich(Level::Mono, true).finish(), "");
}

#[test]
fn leading_blank_lines_are_dropped_but_a_break_after_the_first_word_is_kept() {
    let mut p = rich(Level::Mono, true);
    assert_eq!(
        p.answer("\n\nHello"),
        "",
        "the word is held back until it is complete"
    );
    let mut all = p.answer("\n\n");
    all += &p.answer("World");
    all += &p.finish();
    // the break is not lost just because the word before it had not been written yet
    assert_eq!(strip(&all), "\r\u{1b}[KHello\n\nWorld\n");
}

#[test]
fn a_notice_is_a_coloured_symbol_the_text_a_dim_status_and_dim_detail() {
    let mut p = rich(Level::Ansi16, true);
    let out = p.notice(&notice());
    assert!(out.contains("\u{1b}[32m"), "the symbol is green: {out:?}");
    assert!(
        out.contains("\u{1b}[2m[exit 0, 0.0s]\u{1b}[0m"),
        "the status is one dim run: {out:?}"
    );
    assert!(
        out.contains("\u{1b}[2mone\u{1b}[0m"),
        "so is the detail: {out:?}"
    );
    assert_eq!(
        strip(&out),
        "\r\u{1b}[K✔ ran (approved): ls [exit 0, 0.0s]\n  one\n  two\n"
    );
}

#[test]
fn the_tone_of_a_notice_sets_the_colour_of_its_symbol() {
    for (tone, code) in [
        (Tone::Warning, "33"),
        (Tone::Error, "31"),
        (Tone::Info, "36"),
        (Tone::Success, "32"),
    ] {
        let mut p = rich(Level::Ansi16, true);
        let out = p.notice(&Notice::new(tone, Mark::Cross, "x"));
        assert!(
            out.contains(&format!("\u{1b}[{code}m"))
                || out.contains(&format!("\u{1b}[1m\u{1b}[{code}m")),
            "{tone:?} {out:?}"
        );
    }
}

#[test]
fn a_notice_ends_the_markdown_line_before_it_and_starts_on_its_own_line() {
    let mut p = rich(Level::Mono, true);
    let mut out = p.answer("half **a line");
    out += &p.notice(&Notice::new(
        Tone::Warning,
        Mark::Chevron,
        "denied: exec ls",
    ));
    out += &p.answer("after\n");
    out += &p.finish();
    assert_eq!(
        strip(&out),
        "\r\u{1b}[Khalf **a line\n> denied: exec ls\nafter\n"
    );
}

#[test]
fn a_notice_with_nothing_before_it_does_not_start_with_a_blank_line() {
    let mut p = rich(Level::Ansi16, true);
    let mut n = Notice::new(Tone::Info, Mark::Arrow, "moved to your prompt: ls");
    n.gap = true;
    assert_eq!(
        strip(&p.notice(&n)),
        "\r\u{1b}[K→ moved to your prompt: ls\n"
    );
}

#[test]
fn remarks_list_the_other_options_as_code_and_label_the_rest() {
    let mut p = rich(Level::Ansi16, true);
    p.answer("Answer.\n");
    let r = [
        Remark::new(Tone::Info, "Other options:").with_items(vec!["ls -a".into()]),
        Remark::labelled(Tone::Warning, "check", "`x` was not found"),
    ];
    let out = p.remarks(&r);
    assert!(
        out.contains("\u{1b}[35mls -a\u{1b}[0m"),
        "options look like code: {out:?}"
    );
    assert!(
        out.contains("\u{1b}[33mcheck:\u{1b}[0m"),
        "the label is yellow: {out:?}"
    );
    assert_eq!(
        strip(&out),
        "\nOther options:\n  ls -a\n\ncheck: `x` was not found\n"
    );
    assert_eq!(rich(Level::Ansi16, true).remarks(&[]), "");
}

#[test]
fn an_error_is_red_with_its_hint_on_the_next_line() {
    let e = Error::new("provider.auth", "HTTP 401").with_hint("check the key");
    let out = rich(Level::Ansi16, true).error(&e);
    assert!(
        out.contains("\u{1b}[1m\u{1b}[31m") || out.contains("\u{1b}[31m"),
        "{out:?}"
    );
    assert_eq!(strip(&out), "✘ provider.auth: HTTP 401\n↳ check the key\n");
    let ascii = RichPresenter::new(Caps {
        unicode: false,
        ..caps(Level::Mono, true)
    })
    .error(&e);
    assert_eq!(
        strip(&ascii),
        "x provider.auth: HTTP 401\n> check the key\n"
    );
}

#[test]
fn a_long_error_wraps_with_its_continuation_indented() {
    let e = Error::new("x.y", "word ".repeat(20));
    let out = strip(&rich(Level::Ansi16, true).error(&e));
    assert!(out.lines().all(|l| l.chars().count() <= 40), "{out:?}");
    assert!(out.lines().skip(1).all(|l| l.starts_with("  ")), "{out:?}");
}

#[test]
fn the_waiting_indicator_is_coloured_and_redraws_in_place() {
    let p = rich(Level::Ansi16, true);
    let a = p.spinner(0);
    assert!(
        a.starts_with('\r') && a.contains("thinking…") && a.contains("\u{1b}["),
        "{a:?}"
    );
    assert_eq!(strip(&a), "\r⠋ thinking…");
    assert_ne!(
        p.spinner(0),
        p.spinner(3),
        "the colour changes every few steps"
    );
    assert_eq!(p.clear_spinner(), "\r\u{1b}[K");
    let mono = rich(Level::Mono, true).spinner(1);
    assert!(
        !mono.contains("\u{1b}[3") && !mono.contains("\u{1b}[9"),
        "{mono:?}"
    );
}

#[test]
fn a_status_line_takes_the_look_of_its_tone() {
    let p = rich(Level::Ansi16, true);
    assert_eq!(
        strip(&p.status(Tone::Muted, "[cancelled]")),
        "[cancelled]\n"
    );
    assert!(p.status(Tone::Warning, "careful").contains("\u{1b}[33m"));
}

#[test]
fn a_label_is_escaped_so_that_only_the_theme_writes_escapes() {
    let out = rich(Level::Ansi16, true).label(Tone::Error, "FAIL \u{1b}[2J");
    assert!(only_styles(&out), "{out:?}");
    assert!(out.contains("\\u{1b}[2J"));
    assert_eq!(
        PlainPresenter::new(Caps::plain(40, true)).label(Tone::Error, "FAIL"),
        "FAIL"
    );
}

#[test]
fn a_plain_error_line_has_the_same_text_with_or_without_style() {
    let e = Error::new("config.invalid", "line one\nline two").with_hint("fix it");
    let plain = PlainPresenter::new(Caps::plain(40, true)).cli_error(&e);
    assert_eq!(plain, "config.invalid: line one\nline two\n  fix it\n");
    assert_eq!(strip(&rich(Level::Ansi16, true).cli_error(&e)), plain);
}

#[test]
fn nothing_the_model_writes_can_add_an_escape_to_any_output() {
    let hostile = "x \u{1b}[2J \u{1b}]0;title\u{7} \u{202e} **b \u{1b}[31m** `c\u{1b}[0m` # h\u{1b}[H\n- i \u{1b}c\n```\ncode \u{1b}[2J\n```\n";
    for level in [Level::Mono, Level::Ansi16] {
        let mut p = rich(level, true);
        let mut out = p.answer(hostile);
        out += &p.notice(
            &Notice::new(Tone::Warning, Mark::Chevron, "n \u{1b}[2J")
                .with_detail(vec!["d \u{1b}[2J".into()]),
        );
        out += &p.finish();
        out += &p.remarks(&[Remark::labelled(Tone::Warning, "check", "r \u{1b}[2J")
            .with_items(vec!["i \u{1b}[2J".into()])]);
        out += &p.error(&Error::new("c.d", "m \u{1b}[2J").with_hint("h \u{1b}[2J"));
        out += &p.status(Tone::Info, "s \u{1b}[2J");
        assert!(only_styles(&out), "{level:?}: {out:?}");
    }
}

#[test]
fn mono_has_styles_but_no_colour() {
    let mut p = rich(Level::Mono, true);
    let mut out = p.answer("# T\n**b** *i* `c`\n- x\n");
    out += &p.notice(&notice());
    out += &p.finish();
    assert!(out.contains("\u{1b}[1m"), "{out:?}");
    let colour = sgr_params(&out)
        .iter()
        .any(|n| (30..=37).contains(n) || (90..=97).contains(n));
    assert!(!colour, "{out:?}");
}

#[test]
fn without_markdown_a_notice_still_starts_on_its_own_line() {
    let mut p = rich(Level::Mono, false);
    let mut out = p.answer("half a line");
    out += &p.notice(&Notice::new(
        Tone::Warning,
        Mark::Chevron,
        "denied: exec ls",
    ));
    assert_eq!(strip(&out), "\r\u{1b}[Khalf a line\n> denied: exec ls\n");
}

#[test]
fn it_knows_whether_anything_was_written() {
    let mut p = rich(Level::Ansi16, true);
    assert!(!p.wrote_any());
    p.answer("x\n");
    assert!(p.wrote_any());
}

#[test]
fn the_bullet_follows_the_locale() {
    assert_eq!(rich(Level::Ansi16, true).bullet(), "\u{2022}");
    let ascii = RichPresenter::new(Caps {
        unicode: false,
        ..caps(Level::Ansi16, true)
    });
    assert_eq!(ascii.bullet(), "*");
}

#[test]
fn the_plain_level_gets_the_plain_presenter_even_if_markdown_is_asked_for() {
    let mut p = crate::present::presenter(caps(Level::Plain, true));
    assert_eq!(p.answer("a **b**\n"), "\r\u{1b}[Ka **b**\n");
    let mut r = crate::present::presenter(caps(Level::Mono, true));
    assert_eq!(strip(&r.answer("a **b**\n")), "\r\u{1b}[Ka b\n");
}

#[test]
fn markdown_needs_a_level_with_styles() {
    use crate::ports::MapEnv;
    use crate::present::ColorSetting;
    let term = MapEnv::new().with("TERM", "xterm");
    let mono = MapEnv::new().with("TERM", "xterm").with("NO_COLOR", "1");
    let md = |env: &MapEnv, tty, setting, want| Caps::detect(env, tty, 80, setting, want).markdown;
    assert!(md(&term, true, ColorSetting::Auto, true));
    assert!(md(&mono, true, ColorSetting::Auto, true));
    assert!(!md(&term, true, ColorSetting::Auto, false));
    assert!(!md(&term, false, ColorSetting::Auto, true));
    assert!(!md(&term, true, ColorSetting::Never, true));
    assert!(!md(&term, false, ColorSetting::Never, false));
}
