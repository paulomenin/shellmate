//! A keyboard shortcut as the config writes it (`alt+m`, `ctrl+alt+x`), independent of any shell. The shell
//! glue turns it into fish, zsh or readline notation; here it is only parsed and checked, once, when the
//! config is validated, so `config check` and `init` cannot disagree about what is valid.

use shellmate_api::{Error, Result};

/// A shell-neutral key: a letter or digit with ctrl and/or alt. Chords are not supported yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub ctrl: bool,
    pub alt: bool,
    /// Lower case.
    pub ch: char,
}

impl Shortcut {
    pub fn parse(spec: &str) -> Result<Self> {
        if spec.contains(' ') {
            return Err(Error::new(
                "config.unsupported",
                format!("key chords are not supported yet: `{spec}`"),
            ));
        }
        let parts: Vec<&str> = spec.split('+').collect();
        let (key, mods) = parts
            .split_last()
            .ok_or_else(|| Error::new("config.invalid", "empty shortcut"))?;
        let unsupported = || {
            Error::new(
                "config.unsupported",
                format!(
                    "shortcut `{spec}`: only single letters/digits with ctrl/alt are supported yet"
                ),
            )
        };
        let mut chars = key.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            return Err(unsupported());
        };
        if !ch.is_ascii_alphanumeric() {
            return Err(unsupported());
        }
        let mut k = Shortcut {
            ctrl: false,
            alt: false,
            ch: ch.to_ascii_lowercase(),
        };
        for m in mods {
            match *m {
                "ctrl" => k.ctrl = true,
                "alt" => k.alt = true,
                other => {
                    return Err(Error::new(
                        "config.invalid",
                        format!("shortcut `{spec}`: unknown modifier `{other}`"),
                    ))
                }
            }
        }
        if !k.ctrl && !k.alt {
            return Err(Error::new(
                "config.invalid",
                format!("shortcut `{spec}`: needs ctrl or alt (a bare key would break typing)"),
            ));
        }
        Ok(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_shortcuts() {
        assert_eq!(
            Shortcut::parse("ctrl+alt+J").unwrap(),
            Shortcut {
                ctrl: true,
                alt: true,
                ch: 'j'
            }
        );
        assert_eq!(
            Shortcut::parse("alt+m").unwrap(),
            Shortcut {
                ctrl: false,
                alt: true,
                ch: 'm'
            }
        );
        assert!(Shortcut::parse("ctrl+7").is_ok());
    }

    #[test]
    fn unsupported_and_invalid_shortcuts_say_which() {
        for (spec, code, word) in [
            ("ctrl+x ctrl+e", "config.unsupported", "chords"),
            ("alt+;", "config.unsupported", "single letters"),
            ("alt+mm", "config.unsupported", "single letters"),
            ("alt+", "config.unsupported", "single letters"),
            ("", "config.unsupported", "single letters"),
            ("super+m", "config.invalid", "unknown modifier `super`"),
            ("m", "config.invalid", "needs ctrl or alt"),
        ] {
            let e = Shortcut::parse(spec).unwrap_err();
            assert_eq!(e.code, code, "{spec}");
            assert!(e.message.contains(word), "{spec}: {}", e.message);
        }
    }
}
