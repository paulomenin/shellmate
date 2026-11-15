//! `config edit`: open the config file in the user's editor. The caller then runs `config check` on it.
//!
//! The editor is the person's own choice (`$VISUAL`, then `$EDITOR`), started directly and never through a
//! shell, so a line such as `code --wait` works and shell syntax in it does not. Nothing the model produces
//! reaches this command.

use super::config_path;
use shellmate_api::{Error, Result};
use shellmate_core::ports::Env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Used when neither `$VISUAL` nor `$EDITOR` is set.
const FALLBACK_EDITOR: &str = "vi";

/// The editor command line: `$VISUAL`, then `$EDITOR`, then `vi`. Words are split on blanks, and single or
/// double quotes keep a word together.
pub fn editor_argv(env: &dyn Env) -> Result<Vec<String>> {
    let line = env
        .nonempty("VISUAL")
        .filter(|v| !v.trim().is_empty())
        .or_else(|| env.nonempty("EDITOR").filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| FALLBACK_EDITOR.to_string());
    split_words(&line)
}

fn split_words(line: &str) -> Result<Vec<String>> {
    let (mut words, mut cur, mut quote, mut started) = (Vec::new(), String::new(), None, false);
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\'' | '"', None) => {
                quote = Some(c);
                started = true;
            }
            (q, Some(open)) if q == open => quote = None,
            ('\\', None | Some('"')) => {
                cur.push(chars.next().unwrap_or('\\'));
                started = true;
            }
            (c, None) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (c, _) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(Error::new(
            "config.editor_invalid",
            format!("the editor line `{line}` has an unclosed quote"),
        )
        .with_hint("fix $VISUAL or $EDITOR"));
    }
    if started {
        words.push(cur);
    }
    if words.is_empty() {
        return Err(
            Error::new("config.editor_invalid", "the editor line is empty")
                .with_hint("set $VISUAL or $EDITOR to your editor, e.g. EDITOR=nano"),
        );
    }
    Ok(words)
}

/// Run the editor on the config file and return the file's path once the editor exits successfully.
pub fn config_edit(explicit: Option<&Path>, env: &dyn Env) -> Result<PathBuf> {
    let path = config_path(explicit, env);
    if !path.exists() {
        return Err(Error::new(
            "config.missing",
            format!("{} does not exist", path.display()),
        )
        .with_hint("run `shellmate setup` or `shellmate config init` to write one first"));
    }
    let argv = editor_argv(env)?;
    // a path that begins with `-` must not be read as an option
    let target = if path.is_relative() && path.to_string_lossy().starts_with('-') {
        Path::new(".").join(&path)
    } else {
        path.clone()
    };
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .arg(&target)
        .status()
        .map_err(|e| {
            Error::new(
                "config.editor_failed",
                format!("could not start `{}`: {e}", argv[0]),
            )
            .with_hint("set $VISUAL or $EDITOR to an editor that is installed")
        })?;
    if !status.success() {
        return Err(Error::new(
            "config.editor_failed",
            format!("`{}` exited with {status}", argv[0]),
        )
        .with_hint("the file may still have been saved; `shellmate config check` tells"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellmate_core::ports::MapEnv;

    fn argv(env: &MapEnv) -> Vec<String> {
        editor_argv(env).unwrap()
    }

    #[test]
    fn visual_wins_over_editor_and_vi_is_the_last_resort() {
        let both = MapEnv::new()
            .with("VISUAL", "code --wait")
            .with("EDITOR", "nano");
        assert_eq!(argv(&both), ["code", "--wait"]);
        assert_eq!(argv(&MapEnv::new().with("EDITOR", "nano")), ["nano"]);
        assert_eq!(argv(&MapEnv::new()), ["vi"]);
        // blank values are not an editor
        let blank = MapEnv::new().with("VISUAL", "  ").with("EDITOR", "");
        assert_eq!(argv(&blank), ["vi"]);
    }

    #[test]
    fn quotes_keep_a_word_together_and_there_is_no_shell() {
        let words = |l: &str| split_words(l).unwrap();
        assert_eq!(
            words(r#"emacsclient -t -a """#),
            ["emacsclient", "-t", "-a", ""]
        );
        assert_eq!(
            words(r#""/opt/my editor/ed" -n"#),
            ["/opt/my editor/ed", "-n"]
        );
        assert_eq!(words(r"my\ ed 'a b'"), ["my ed", "a b"]);
        // shell syntax is just text: it is never interpreted
        assert_eq!(words("vi; rm -rf ~"), ["vi;", "rm", "-rf", "~"]);
        assert_eq!(words("$(id)"), ["$(id)"]);
    }

    #[test]
    fn a_broken_editor_line_is_an_error() {
        assert!(split_words("vim 'oops").is_err());
        assert!(split_words("   ").is_err());
    }
}
