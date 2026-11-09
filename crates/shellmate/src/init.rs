//! `shellmate init <shell>`: generate the shell glue from the templates in `crates/shells/<shell>/`.

use shellmate_api::{Error, Result};
use shellmate_core::Config;

const FISH: &str = include_str!("../../shells/fish/glue.fish");
const ZSH: &str = include_str!("../../shells/zsh/glue.zsh");
const BASH: &str = include_str!("../../shells/bash/glue.bash");

/// A shell-neutral key: `alt+m`, `ctrl+x`, `ctrl+alt+x`. Chords are not supported yet.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Key {
    ctrl: bool,
    alt: bool,
    ch: char,
}

pub(crate) fn parse_key(spec: &str) -> Result<Key> {
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
    let mut chars = key.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return Err(Error::new(
            "config.unsupported",
            format!(
                "shortcut `{spec}`: only single letters/digits with ctrl/alt are supported yet"
            ),
        ));
    };
    if !ch.is_ascii_alphanumeric() {
        return Err(Error::new(
            "config.unsupported",
            format!(
                "shortcut `{spec}`: only single letters/digits with ctrl/alt are supported yet"
            ),
        ));
    }
    let mut k = Key {
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

impl Key {
    fn fish(&self) -> String {
        format!(
            "{}{}{}",
            if self.ctrl { "ctrl-" } else { "" },
            if self.alt { "alt-" } else { "" },
            self.ch
        )
    }
    /// zsh `bindkey` notation.
    fn zsh(&self) -> String {
        let base = if self.ctrl {
            format!("^{}", self.ch.to_ascii_uppercase())
        } else {
            self.ch.to_string()
        };
        if self.alt {
            format!("^[{base}")
        } else {
            base
        }
    }
    /// readline notation (as printed by `bind -p`).
    fn readline(&self) -> String {
        let base = if self.ctrl {
            format!("\\C-{}", self.ch)
        } else {
            self.ch.to_string()
        };
        if self.alt {
            format!("\\e{base}")
        } else {
            base
        }
    }
}

/// A fresh id for one interactive shell: 16 hex characters from the kernel's random source.
fn new_session() -> Result<String> {
    use std::io::Read;
    let mut b = [0u8; 8];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| Error::new("init.no_entropy", format!("cannot read /dev/urandom: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn check_bin(bin: &str) -> Result<()> {
    if bin.contains('\'') || bin.contains('\n') {
        return Err(Error::new(
            "init.bad_path",
            "the binary path contains characters that cannot be quoted safely",
        ));
    }
    Ok(())
}

fn fish(cfg: &Config, bin: &str) -> Result<String> {
    let mut b = String::new();
    for (spec, action) in &cfg.input.shortcuts {
        let key = parse_key(spec)?.fish();
        // conflict detection: never silently override an existing binding
        b.push_str(&format!(
            "if bind {key} >/dev/null 2>&1\n    echo \"shellmate: {key} is already bound; not binding it to '{action}' (change [input.shortcuts])\" >&2\nelse\n    bind {key} '__shellmate_widget {action}'\n    bind -M insert {key} '__shellmate_widget {action}'\nend\n"
        ));
    }
    Ok(FISH
        .replace("@BIN@", bin)
        .replace("@SESSION@", &new_session()?)
        .replace("@BINDINGS@", b.trim_end()))
}

fn zsh(cfg: &Config, bin: &str) -> Result<String> {
    let mut b = String::new();
    for (spec, action) in &cfg.input.shortcuts {
        let key = parse_key(spec)?.zsh();
        b.push_str(&format!(
            "__shellmate_w_{action}() {{ __shellmate_run {action}; }}\nzle -N __shellmate_w_{action}\nif [[ $(bindkey -M emacs '{key}' 2>/dev/null) != *undefined-key* ]]; then\n  print -u2 \"shellmate: {key} is already bound; not binding it to '{action}' (change [input.shortcuts])\"\nelse\n  for __shellmate_km in emacs viins vicmd; do bindkey -M $__shellmate_km '{key}' __shellmate_w_{action}; done\nfi\n"
        ));
    }
    b.push_str("unset __shellmate_km\n");
    Ok(ZSH
        .replace("@BIN@", bin)
        .replace("@SESSION@", &new_session()?)
        .replace("@BINDINGS@", b.trim_end()))
}

fn bash(cfg: &Config, bin: &str) -> Result<String> {
    let mut b = String::new();
    for (spec, action) in &cfg.input.shortcuts {
        let key = parse_key(spec)?.readline();
        b.push_str(&format!(
            "__shellmate_w_{action}() {{ __shellmate_run {action}; }}\nif [[ $(bind -m emacs -p) == *'\"{key}\":'* ]]; then\n  echo \"shellmate: {key} is already bound; not binding it to '{action}' (change [input.shortcuts])\" >&2\nelse\n  bind -m emacs -x '\"{key}\": __shellmate_w_{action}'\n  bind -m vi-insert -x '\"{key}\": __shellmate_w_{action}'\n  bind -m vi-command -x '\"{key}\": __shellmate_w_{action}'\nfi\n"
        ));
    }
    Ok(BASH
        .replace("@BIN@", bin)
        .replace("@SESSION@", &new_session()?)
        .replace("@BINDINGS@", b.trim_end()))
}

pub fn generate(shell: &str, cfg: &Config, bin: &str) -> Result<String> {
    check_bin(bin)?;
    match shell {
        "fish" => fish(cfg, bin),
        "zsh" => zsh(cfg, bin),
        "bash" => bash(cfg, bin),
        other => Err(
            Error::new("init.unsupported_shell", format!("unknown shell `{other}`"))
                .with_hint("supported: fish, zsh, bash"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_translation() {
        let k = parse_key("alt+m").unwrap();
        assert_eq!(
            (k.fish(), k.zsh(), k.readline()),
            ("alt-m".into(), "^[m".into(), "\\em".into())
        );
        let k = parse_key("ctrl+x").unwrap();
        assert_eq!(
            (k.fish(), k.zsh(), k.readline()),
            ("ctrl-x".into(), "^X".into(), "\\C-x".into())
        );
        let k = parse_key("ctrl+alt+j").unwrap();
        assert_eq!(
            (k.fish(), k.zsh(), k.readline()),
            ("ctrl-alt-j".into(), "^[^J".into(), "\\e\\C-j".into())
        );
    }

    #[test]
    fn bad_keys() {
        assert!(parse_key("ctrl+x ctrl+e").is_err());
        assert!(parse_key("super+m").is_err());
        assert!(parse_key("alt+;").is_err());
        assert!(parse_key("m").is_err(), "a bare letter would break typing");
        assert!(parse_key("alt+mm").is_err());
    }

    #[test]
    fn generated_glue_has_bindings_and_no_placeholders() {
        for shell in ["fish", "zsh", "bash"] {
            let g = generate(shell, &Config::defaults(), "/usr/bin/shellmate").unwrap();
            assert!(g.contains("/usr/bin/shellmate"), "{shell}");
            assert!(
                !g.contains("@BIN@") && !g.contains("@BINDINGS@") && !g.contains("@SESSION@"),
                "{shell}: unreplaced placeholder"
            );
            assert!(g.contains("ask"), "{shell}: binding for ask");
        }
    }

    #[test]
    fn every_init_gets_a_fresh_session_id() {
        for shell in ["fish", "zsh", "bash"] {
            let a = generate(shell, &Config::defaults(), "/x").unwrap();
            let b = generate(shell, &Config::defaults(), "/x").unwrap();
            let id = |g: &str| {
                let i = g.find("SHELLMATE_SESSION").unwrap();
                g[i..].lines().next().unwrap().to_string()
            };
            assert_ne!(id(&a), id(&b), "{shell}");
            assert!(
                a.contains("session=%s"),
                "{shell}: the envelope carries the session"
            );
        }
    }

    #[test]
    fn unknown_shells_are_reported() {
        assert_eq!(
            generate("nu", &Config::defaults(), "/x").unwrap_err().code,
            "init.unsupported_shell"
        );
    }
}
