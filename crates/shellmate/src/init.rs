//! `shellmate init <shell>`: generate the shell glue from the templates in `crates/shells/<shell>/`.

use shellmate_api::{Error, Result};
use shellmate_core::config::Shortcut;
use shellmate_core::Config;

const FISH: &str = include_str!("../../shells/fish/glue.fish");
const ZSH: &str = include_str!("../../shells/zsh/glue.zsh");
const BASH: &str = include_str!("../../shells/bash/glue.bash");

/// The key in fish `bind` notation.
fn fish_key(k: &Shortcut) -> String {
    format!(
        "{}{}{}",
        if k.ctrl { "ctrl-" } else { "" },
        if k.alt { "alt-" } else { "" },
        k.ch
    )
}

/// The key in zsh `bindkey` notation.
fn zsh_key(k: &Shortcut) -> String {
    let base = if k.ctrl {
        format!("^{}", k.ch.to_ascii_uppercase())
    } else {
        k.ch.to_string()
    };
    if k.alt {
        format!("^[{base}")
    } else {
        base
    }
}

/// The key in readline notation (as printed by `bind -p`).
fn readline_key(k: &Shortcut) -> String {
    let base = if k.ctrl {
        format!("\\C-{}", k.ch)
    } else {
        k.ch.to_string()
    };
    if k.alt {
        format!("\\e{base}")
    } else {
        base
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
        let key = fish_key(&Shortcut::parse(spec)?);
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
        let key = zsh_key(&Shortcut::parse(spec)?);
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
        let key = readline_key(&Shortcut::parse(spec)?);
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
        let t = |spec: &str| {
            let k = Shortcut::parse(spec).unwrap();
            (fish_key(&k), zsh_key(&k), readline_key(&k))
        };
        assert_eq!(t("alt+m"), ("alt-m".into(), "^[m".into(), "\\em".into()));
        assert_eq!(t("ctrl+x"), ("ctrl-x".into(), "^X".into(), "\\C-x".into()));
        assert_eq!(
            t("ctrl+alt+j"),
            ("ctrl-alt-j".into(), "^[^J".into(), "\\e\\C-j".into())
        );
    }

    #[test]
    fn config_validation_and_init_agree_on_what_a_shortcut_is() {
        for spec in [
            "ctrl+x ctrl+e",
            "super+m",
            "alt+;",
            "m",
            "alt+mm",
            "alt+",
            "hyper+zz",
        ] {
            let text = format!("version = 1\n[input.shortcuts]\n\"{spec}\" = \"ask\"\n");
            let err = Config::parse(&text).expect_err(spec);
            assert!(
                err.message.starts_with("input.shortcuts: "),
                "{spec}: {}",
                err.message
            );
        }
        for spec in ["alt+m", "ctrl+x", "ctrl+alt+j", "alt+7"] {
            let text = format!("version = 1\n[input.shortcuts]\n\"{spec}\" = \"ask\"\n");
            let cfg = Config::parse(&text).expect(spec);
            for shell in ["fish", "zsh", "bash"] {
                assert!(
                    generate(shell, &cfg, "/bin/shellmate").is_ok(),
                    "{shell} {spec}"
                );
            }
        }
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
