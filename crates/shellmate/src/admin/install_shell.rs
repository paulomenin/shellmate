//! `install-shell`: add, update or remove the shellmate block in the shell's rc file.

use super::home;
use shellmate_api::{Error, Result};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

pub(super) const BEGIN: &str = "# >>> shellmate >>>";
const END: &str = "# <<< shellmate <<<";

pub(super) fn rc_path(shell: &str) -> Result<PathBuf> {
    Ok(match shell {
        "fish" => std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home().join(".config"))
            .join("fish/config.fish"),
        "zsh" => std::env::var_os("ZDOTDIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join(".zshrc"),
        "bash" => home().join(".bashrc"),
        other => {
            return Err(
                Error::new("init.unsupported_shell", format!("unknown shell `{other}`"))
                    .with_hint("supported: fish, zsh, bash"),
            )
        }
    })
}

fn init_line(shell: &str, bin: &str) -> String {
    match shell {
        "fish" => format!("'{bin}' init fish | source"),
        "zsh" => format!("source <('{bin}' init zsh)"),
        _ => format!("eval \"$('{bin}' init bash)\""),
    }
}

/// The rc text with the shellmate block added, updated or (with `remove`) deleted.
pub fn edit_rc(text: &str, shell: &str, bin: &str, remove: bool) -> Result<String> {
    if bin.contains(['\'', '\n']) {
        return Err(Error::new(
            "init.bad_path",
            "the binary path contains characters that cannot be quoted safely",
        ));
    }
    let block = format!("{BEGIN}\n{}\n{END}\n", init_line(shell, bin));
    if let (Some(b), Some(e)) = (text.find(BEGIN), text.find(END)) {
        if b < e {
            let end = e + END.len();
            let end = if text[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            let mut out = text[..b].to_string();
            if !remove {
                out.push_str(&block);
            }
            out.push_str(&text[end..]);
            return Ok(out);
        }
    }
    if remove {
        return Ok(text.to_string());
    }
    let mut out = text.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&block); // at the end: bash needs to come after prompt tools
    Ok(out)
}

pub(super) fn confirm(question: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(
            Error::new("setup.needs_tty", "this needs your confirmation")
                .with_hint("run it in a terminal, or pass --yes"),
        );
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

pub fn install_shell(shell: &str, yes: bool, remove: bool, dry_run: bool) -> Result<String> {
    let bin = std::env::current_exe()
        .map_err(|e| Error::new("init.bad_path", e.to_string()))?
        .to_string_lossy()
        .into_owned();
    let rc = rc_path(shell)?;
    let old = std::fs::read_to_string(&rc).unwrap_or_default();
    let new = edit_rc(&old, shell, &bin, remove)?;
    if new == old {
        return Ok(format!("{}: nothing to change\n", rc.display()));
    }
    let mut out = format!(
        "{} in {}:\n{}\n",
        if remove { "removing" } else { "adding" },
        rc.display(),
        new.lines()
            .skip_while(|l| *l != BEGIN)
            .take_while(|l| *l != END)
            .chain(std::iter::once(END))
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    if remove {
        out = format!("removing the shellmate block from {}\n", rc.display());
    }
    if dry_run {
        return Ok(out);
    }
    print!("{out}");
    if !confirm("apply?", yes)? {
        return Ok("not changed\n".into());
    }
    if let Some(dir) = rc.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if rc.exists() {
        let backup = rc.with_file_name(format!(
            "{}.shellmate-backup",
            rc.file_name().unwrap_or_default().to_string_lossy()
        ));
        if !backup.exists() {
            std::fs::copy(&rc, &backup)
                .map_err(|e| Error::new("setup.write_failed", format!("backup: {e}")))?;
        }
    }
    let tmp = rc.with_extension("shellmate-tmp");
    std::fs::write(&tmp, &new)
        .and_then(|_| std::fs::rename(&tmp, &rc))
        .map_err(|e| Error::new("setup.write_failed", format!("{}: {e}", rc.display())))?;
    Ok(format!(
        "{} {}\nopen a new shell to use it\n",
        if remove { "updated" } else { "installed into" },
        rc.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rc_block_is_added_once_updated_in_place_and_removed() {
        let a = edit_rc("export A=1", "bash", "/bin/sm", false).unwrap();
        assert!(a.starts_with("export A=1\n") && a.contains("eval \"$('/bin/sm' init bash)\""));
        assert!(a.trim_end().ends_with(END), "bash block goes last");
        // idempotent
        assert_eq!(edit_rc(&a, "bash", "/bin/sm", false).unwrap(), a);
        // moved binary: updated in place, surrounding text kept
        let with_tail = format!("{a}export B=2\n");
        let b = edit_rc(&with_tail, "bash", "/opt/sm", false).unwrap();
        assert!(b.contains("/opt/sm") && !b.contains("/bin/sm") && b.contains("export B=2"));
        assert_eq!(b.matches(BEGIN).count(), 1);
        // remove restores the original content
        let r = edit_rc(&a, "bash", "/bin/sm", true).unwrap();
        assert_eq!(r.trim(), "export A=1");
        assert_eq!(edit_rc("x\n", "fish", "/b", true).unwrap(), "x\n");
    }

    #[test]
    fn rc_lines_per_shell() {
        assert!(edit_rc("", "fish", "/b", false)
            .unwrap()
            .contains("'/b' init fish | source"));
        assert!(edit_rc("", "zsh", "/b", false)
            .unwrap()
            .contains("source <('/b' init zsh)"));
        assert!(edit_rc("", "bash", "/b'x", false).is_err());
    }
}
