//! A command ready to run, and finding its program on a `PATH`.

use std::path::{Path, PathBuf};

/// Everything about a command that `exec` needs: the resolved program and the arguments. Approval, the
/// prompt and the execution all use this one value, so what the user saw is what runs (`approval-ux.md` §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// Absolute path, resolved through `PATH` before anything is matched.
    pub program: PathBuf,
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
}

impl Prepared {
    pub fn display(&self) -> String {
        display_argv(&self.argv)
    }
}

/// The words of a command as one line for a person to read: a word with spaces or quotes in it is quoted.
pub fn display_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|w| {
            if w.is_empty() || w.contains([' ', '\t', '\'', '"']) {
                format!("'{}'", w.replace('\'', "'\\''"))
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve `name` through `path` (a `PATH` value). Names containing `/` are never resolved: `./ls` and
/// `/tmp/x/ls` do not count as the command `ls`.
pub fn resolve_program(name: &str, path: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    if name.is_empty() || name.contains('/') {
        return None;
    }
    path.split(':')
        .filter(|d| Path::new(d).is_absolute())
        .map(|d| Path::new(d).join(name))
        .find(|p| {
            p.is_file()
                && p.metadata()
                    .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
        })
}
