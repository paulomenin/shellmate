//! How much a terminal can show, decided once from the environment.
//!
//! The decision is a pure function of the environment, whether there is a terminal, and the user's setting,
//! so every combination can be tested without one.

use crate::ports::Env;
use serde::Deserialize;

/// How rich the output may be. Everything is drawn once at the richest level and reduced to the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Text only: no escape sequences at all, and the model's markdown is left as written.
    Plain,
    /// Bold, italic, underline and dim, but no colour (what `NO_COLOR` asks for).
    Mono,
    /// The 16 ANSI colours, which follow the terminal's own theme.
    Ansi16,
}

/// The user's choice, `[ui] color` in the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorSetting {
    /// Decide from the terminal and the environment.
    #[default]
    Auto,
    /// Always use colour, even when output is not a terminal.
    Always,
    /// Never use colour or styling.
    Never,
}

/// Whether the locale says the terminal can show Unicode.
pub fn locale_is_utf8(env: &dyn Env) -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| env.nonempty(k))
        .is_some_and(|v| {
            let v = v.to_lowercase();
            v.contains("utf-8") || v.contains("utf8")
        })
}

/// Which level to use. `is_terminal` says whether the stream being written to is a terminal.
///
/// An explicit `never` or `always` in the config wins over everything. In `auto`, in this order:
/// - `NO_COLOR` set to anything but empty wins over `CLICOLOR_FORCE` and allows only styling, not colour;
/// - `CLICOLOR_FORCE` (not `0`) asks for colour even without a terminal;
/// - no terminal, or `TERM` unset or `dumb`: plain text;
/// - `CLICOLOR=0`: styling without colour;
/// - otherwise colour.
pub fn detect_level(env: &dyn Env, is_terminal: bool, setting: ColorSetting) -> Level {
    match setting {
        ColorSetting::Never => return Level::Plain,
        ColorSetting::Always => return Level::Ansi16,
        ColorSetting::Auto => {}
    }
    let no_color = env.nonempty("NO_COLOR").is_some();
    let forced = env.nonempty("CLICOLOR_FORCE").is_some_and(|v| v != "0");
    if forced && !no_color {
        return Level::Ansi16;
    }
    let dumb = env.var("TERM").is_none_or(|t| t.is_empty() || t == "dumb");
    if !is_terminal || dumb {
        return Level::Plain;
    }
    if no_color || env.var("CLICOLOR").as_deref() == Some("0") {
        return Level::Mono;
    }
    Level::Ansi16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::MapEnv;

    fn term() -> MapEnv {
        MapEnv::new().with("TERM", "xterm-256color")
    }

    fn level(env: &MapEnv, tty: bool, s: ColorSetting) -> Level {
        detect_level(env, tty, s)
    }

    #[test]
    fn a_terminal_with_a_real_term_gets_colour() {
        assert_eq!(level(&term(), true, ColorSetting::Auto), Level::Ansi16);
    }

    #[test]
    fn without_a_terminal_or_with_a_dumb_one_it_is_plain_text() {
        assert_eq!(level(&term(), false, ColorSetting::Auto), Level::Plain);
        for dumb in ["dumb", ""] {
            let env = MapEnv::new().with("TERM", dumb);
            assert_eq!(
                level(&env, true, ColorSetting::Auto),
                Level::Plain,
                "{dumb:?}"
            );
        }
        assert_eq!(
            level(&MapEnv::new(), true, ColorSetting::Auto),
            Level::Plain,
            "TERM unset"
        );
    }

    #[test]
    fn no_color_removes_colour_but_keeps_styling() {
        let env = term().with("NO_COLOR", "1");
        assert_eq!(level(&env, true, ColorSetting::Auto), Level::Mono);
        // an empty NO_COLOR does not count (no-color.org)
        let empty = term().with("NO_COLOR", "");
        assert_eq!(level(&empty, true, ColorSetting::Auto), Level::Ansi16);
        // and still plain when there is no terminal
        assert_eq!(level(&env, false, ColorSetting::Auto), Level::Plain);
    }

    #[test]
    fn clicolor_zero_removes_colour_and_clicolor_force_adds_it_back_without_a_terminal() {
        let off = term().with("CLICOLOR", "0");
        assert_eq!(level(&off, true, ColorSetting::Auto), Level::Mono);
        let force = MapEnv::new().with("CLICOLOR_FORCE", "1");
        assert_eq!(level(&force, false, ColorSetting::Auto), Level::Ansi16);
        let not_forced = MapEnv::new().with("CLICOLOR_FORCE", "0");
        assert_eq!(level(&not_forced, false, ColorSetting::Auto), Level::Plain);
    }

    #[test]
    fn no_color_wins_over_clicolor_force() {
        let env = term().with("NO_COLOR", "1").with("CLICOLOR_FORCE", "1");
        assert_eq!(level(&env, true, ColorSetting::Auto), Level::Mono);
        assert_eq!(level(&env, false, ColorSetting::Auto), Level::Plain);
    }

    #[test]
    fn the_config_wins_over_the_environment() {
        let hostile = MapEnv::new().with("NO_COLOR", "1").with("TERM", "dumb");
        assert_eq!(level(&hostile, false, ColorSetting::Always), Level::Ansi16);
        let friendly = term().with("CLICOLOR_FORCE", "1");
        assert_eq!(level(&friendly, true, ColorSetting::Never), Level::Plain);
    }

    #[test]
    fn the_levels_are_ordered_from_least_to_most() {
        assert!(Level::Plain < Level::Mono && Level::Mono < Level::Ansi16);
    }

    #[test]
    fn unicode_follows_the_first_locale_variable_that_is_set() {
        let utf8 = |k: &str, v: &str| locale_is_utf8(&MapEnv::new().with(k, v));
        assert!(utf8("LANG", "en_US.UTF-8"));
        assert!(utf8("LC_ALL", "C.utf8"));
        assert!(!utf8("LANG", "C"));
        assert!(!locale_is_utf8(&MapEnv::new()));
        // LC_ALL beats LANG, as in the C library
        let env = MapEnv::new()
            .with("LC_ALL", "C")
            .with("LANG", "en_US.UTF-8");
        assert!(!locale_is_utf8(&env));
        // an empty variable is skipped
        let env = MapEnv::new().with("LC_ALL", "").with("LANG", "en_US.UTF-8");
        assert!(locale_is_utf8(&env));
    }

    #[test]
    fn the_setting_parses_from_its_lowercase_names() {
        #[derive(Deserialize)]
        struct T {
            c: ColorSetting,
        }
        let parse = |s: &str| toml::from_str::<T>(&format!("c = \"{s}\"")).map(|t| t.c);
        assert_eq!(parse("auto").unwrap(), ColorSetting::Auto);
        assert_eq!(parse("always").unwrap(), ColorSetting::Always);
        assert_eq!(parse("never").unwrap(), ColorSetting::Never);
        assert!(parse("sometimes").is_err());
    }
}
