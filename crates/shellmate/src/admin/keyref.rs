//! Where an API key lives, as `setup` asks for it. Only references are accepted, never the key itself.

use shellmate_api::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRef {
    Env(String),
    File(String),
}

impl KeyRef {
    /// `env:NAME`, `file:PATH`, or a bare variable name such as `MY_API_KEY`. Anything else is refused, and
    /// the refusal does not repeat the input: it may be a key that was pasted by mistake.
    pub fn parse(input: &str) -> Result<KeyRef> {
        let t = input.trim();
        let var = |n: &str| {
            let mut c = n.chars();
            c.next()
                .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
                && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
        };
        if let Some(n) = t.strip_prefix("env:") {
            if var(n) {
                return Ok(KeyRef::Env(n.to_string()));
            }
        } else if let Some(p) = t.strip_prefix("file:") {
            if !p.trim().is_empty()
                && !p.contains(['\n', '"', '\\'])
                && !p.chars().any(char::is_control)
            {
                return Ok(KeyRef::File(p.trim().to_string()));
            }
        } else if t.contains('_')
            && var(t)
            && t.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            // an upper-case name with an underscore reads as a variable, not as a key
            return Ok(KeyRef::Env(t.to_string()));
        }
        Err(
            Error::new("setup.bad_key_ref", "that is not a reference to a key").with_hint(
                "write where the key is, `env:NAME` or `file:/path/to/key`, never the key itself",
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_and_bare_variable_names_are_accepted() {
        assert_eq!(
            KeyRef::parse("env:MY_KEY").unwrap(),
            KeyRef::Env("MY_KEY".into())
        );
        assert_eq!(
            KeyRef::parse("  OPENROUTER_API_KEY ").unwrap(),
            KeyRef::Env("OPENROUTER_API_KEY".into())
        );
        assert_eq!(
            KeyRef::parse("file:~/.keys/openai").unwrap(),
            KeyRef::File("~/.keys/openai".into())
        );
    }

    #[test]
    fn anything_that_could_be_a_key_is_refused_without_being_repeated() {
        for typed in [
            "sk-or-v1-abcdef0123456789",
            "sk-abc",
            "ABCDEF0123456789",
            "abc_def",
            "env:",
            "env:has space",
            "env:BAD-NAME",
            "file:",
            "file:  ",
            "",
            "cmd:pass show x",
        ] {
            let e = KeyRef::parse(typed).unwrap_err();
            assert_eq!(e.code, "setup.bad_key_ref", "{typed:?}");
            let shown = format!("{} {}", e.message, e.hint.unwrap_or_default());
            assert!(
                matches!(typed, "" | "env:" | "file:") || !shown.contains(typed),
                "{typed:?} echoed: {shown}"
            );
        }
    }
}
