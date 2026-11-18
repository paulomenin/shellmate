//! The `read` tool: paths checked against the allowed roots, plain text files only.

use super::text::strip_terminal_codes;
use shellmate_api::{FileTarget, FileText};

fn expand_home(arg: &str, home: Option<&str>) -> Option<std::path::PathBuf> {
    match arg.strip_prefix("~/") {
        Some(rest) => home
            .filter(|h| !h.is_empty())
            .map(|h| std::path::Path::new(h).join(rest)),
        None => Some(std::path::PathBuf::from(arg)),
    }
}

/// Resolve and vet the path in `arg`. The target is canonical (symlinks resolved), inside an allowed root, a
/// regular file, and not credential-looking. `Err` carries the reason, which goes back to the model.
pub fn prepare_read(
    arg: &str,
    cwd: Option<&str>,
    home: Option<&str>,
    extra_roots: &[String],
) -> Result<FileTarget, String> {
    let argv = crate::command::parse_strict(arg)
        .map_err(|e| format!("write the path as one plain word ({})", e.describe()))?;
    let [given] = argv.as_slice() else {
        return Err("read takes exactly one path, like `read Cargo.toml`".into());
    };
    let cwd_path = cwd
        .map(std::path::Path::new)
        .filter(|c| c.is_dir())
        .ok_or("there is no working directory to read from")?;
    let wanted = expand_home(given, home).ok_or("`~` needs a home directory")?;
    let joined = if wanted.is_absolute() {
        wanted
    } else {
        cwd_path.join(wanted)
    };
    // One message for "missing" and "outside": the model must not be able to probe which paths exist
    // beyond the directories it may read.
    let unavailable = |extra: bool| {
        format!(
            "`{given}` does not exist or is outside the directories this tool may read (the working directory{}). \
             Ask the user to run a command, or to add the directory to `tools.read.roots`.",
            if extra { " and the configured roots" } else { "" }
        )
    };
    let path = std::fs::canonicalize(&joined).map_err(|_| unavailable(!extra_roots.is_empty()))?;
    let cwd_root = std::fs::canonicalize(cwd_path).map_err(|e| e.to_string())?;
    let mut roots = vec![cwd_root.clone()];
    for r in extra_roots {
        if let Some(p) = expand_home(r, home).and_then(|p| std::fs::canonicalize(p).ok()) {
            roots.push(p);
        }
    }
    let Some(root) = roots.iter().find(|r| path.starts_with(r)) else {
        return Err(unavailable(!extra_roots.is_empty()));
    };
    let rel = path
        .strip_prefix(root)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(why) = crate::command::sensitive_reason(&rel, &path.to_string_lossy()) {
        return Err(format!(
            "`{given}` looks like credentials or system data (matches `{why}`); it is never read. \
             Continue without it."
        ));
    }
    let meta = std::fs::metadata(&path).map_err(|e| format!("cannot read `{given}`: {e}"))?;
    if !meta.is_file() {
        return Err(format!(
            "`{given}` is not a regular file; to list a directory, run `exec ls {given}`"
        ));
    }
    let shown = path
        .strip_prefix(&cwd_root)
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    Ok(FileTarget {
        path,
        shown,
        size: meta.len(),
    })
}

/// Read up to `max` bytes of text. Binary files are refused; control bytes are removed.
pub fn read_file(t: &FileTarget, max: usize) -> Result<FileText, String> {
    use std::io::Read;
    let file =
        std::fs::File::open(&t.path).map_err(|e| format!("cannot open `{}`: {e}", t.shown))?;
    // the opened file, not the path, decides: it must still be a regular file
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(format!("`{}` is not a regular file", t.shown));
    }
    let mut buf = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("cannot read `{}`: {e}", t.shown))?;
    if buf.iter().take(8192).any(|b| *b == 0) {
        return Err(format!("`{}` is a binary file", t.shown));
    }
    let truncated = buf.len() > max;
    buf.truncate(max);
    let text = strip_terminal_codes(&String::from_utf8_lossy(&buf));
    Ok(FileText {
        text,
        bytes: meta.len(),
        truncated,
    })
}
