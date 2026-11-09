//! The risk classifier. It can only make things stricter: `Safe` approves nothing by itself.

use super::parse::parse_strict;
use super::tables::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing known to be dangerous. Approves nothing by itself.
    Safe,
    /// Must not run without a human decision.
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub verdict: Verdict,
    pub risk: Risk,
    pub reasons: Vec<String>,
}

impl Assessment {
    fn safe() -> Self {
        Self {
            verdict: Verdict::Safe,
            risk: Risk::Low,
            reasons: Vec::new(),
        }
    }
    fn flag(&mut self, risk: Risk, reason: impl Into<String>) {
        self.verdict = Verdict::Unsafe;
        self.risk = self.risk.max(risk);
        self.reasons.push(reason.into());
    }
}

fn base(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// Rate a command line. Complex lines (pipes, redirects, substitutions...) are unsafe by definition; the
/// words in them are still looked at so the reasons say what is dangerous.
pub fn classify(text: &str) -> Assessment {
    let mut a = Assessment::safe();
    match parse_strict(text) {
        Ok(argv) => classify_argv(&argv, &mut a),
        Err(why) => {
            a.flag(
                Risk::Medium,
                format!("not a simple command: {}", why.describe()),
            );
            // Look at every word anyway, split on anything that is not part of a word.
            let words: Vec<String> = text
                .split(|c: char| !(c.is_ascii_alphanumeric() || "_./-".contains(c)))
                .filter(|w| !w.is_empty())
                .map(str::to_string)
                .collect();
            for w in &words {
                let b = base(w);
                if FILE_DESTROYERS.contains(&b)
                    || PRIVILEGE.contains(&b)
                    || PROCESS_CONTROL.contains(&b)
                {
                    a.flag(Risk::High, format!("contains `{b}`"));
                }
            }
            sensitive_paths(&words, &mut a);
        }
    }
    a.reasons.dedup();
    a
}

/// Why `path` looks like credentials, if it does. `rel` is the part below the directory the user opened up
/// (names are checked there, so a project that happens to be called `secrets-app` is not blocked); location
/// patterns (those starting with `/`, like `/.ssh`) are also checked against the whole `full` path.
pub fn sensitive_reason(rel: &str, full: &str) -> Option<&'static str> {
    let rel = format!("/{}", rel.trim_start_matches('/').to_ascii_lowercase());
    let full = format!("/{}", full.trim_start_matches('/').to_ascii_lowercase());
    SENSITIVE
        .iter()
        .find(|s| rel.contains(**s) || (s.starts_with('/') && full.contains(**s)))
        .copied()
}

fn sensitive_paths(words: &[String], a: &mut Assessment) {
    for w in words {
        let lower = format!("/{}", w.to_ascii_lowercase());
        if SENSITIVE.iter().any(|s| lower.contains(s)) {
            a.flag(Risk::High, format!("touches a sensitive path: {w}"));
        }
    }
}

fn classify_argv(argv: &[String], a: &mut Assessment) {
    let prog = base(&argv[0]);
    let args = &argv[1..];
    let has = |pred: &dyn Fn(&str) -> bool| args.iter().any(|x| pred(x));
    if FILE_DESTROYERS.contains(&prog) || prog.starts_with("mkfs") {
        a.flag(Risk::High, format!("`{prog}` modifies or destroys files"));
    }
    if PRIVILEGE.contains(&prog) {
        a.flag(
            Risk::High,
            format!("`{prog}` runs commands with elevated privileges"),
        );
    }
    if PROCESS_CONTROL.contains(&prog) {
        a.flag(
            Risk::High,
            format!("`{prog}` stops processes or the system"),
        );
    }
    if RUNNERS.contains(&prog) {
        a.flag(
            Risk::High,
            format!("`{prog}` runs other programs or arbitrary code"),
        );
    }
    if NETWORK.contains(&prog) {
        a.flag(
            Risk::Medium,
            format!("`{prog}` sends or fetches data over the network"),
        );
    }
    match prog {
        "systemctl" | "service" | "loginctl" | "machinectl" => {
            let verb = args
                .iter()
                .find(|x| !x.starts_with('-'))
                .map(String::as_str);
            let ok = match prog {
                "service" => args.get(1).is_some_and(|v| v == "status"),
                _ => verb.is_some_and(|v| SERVICE_READS.contains(&v)) || args.is_empty(),
            };
            if !ok {
                a.flag(
                    Risk::High,
                    format!("`{prog}` may change services or the system"),
                );
            }
        }
        "find"
            if has(&|x| {
                matches!(
                    x,
                    "-exec"
                        | "-execdir"
                        | "-ok"
                        | "-okdir"
                        | "-delete"
                        | "-fprint"
                        | "-fprintf"
                        | "-fls"
                )
            }) =>
        {
            a.flag(
                Risk::High,
                "`find` with an action that runs programs or changes files",
            )
        }
        "sed" if has(&|x| x == "-i" || x.starts_with("-i") || x.starts_with("--in-place")) => {
            a.flag(Risk::High, "`sed -i` edits files")
        }
        "sed"
            if has(&|x| {
                x.contains(";e")
                    || x.starts_with('e')
                    || x.contains("/e")
                    || x.starts_with('w')
                    || x.contains(";w")
            }) =>
        {
            a.flag(Risk::Medium, "`sed` script may run commands or write files")
        }
        "git" => {
            if has(&|x| {
                x == "-c"
                    || x.starts_with("--exec-path")
                    || x == "--git-dir"
                    || x.starts_with("--config-env")
            }) {
                a.flag(Risk::High, "`git -c` can run arbitrary programs");
            }
            let sub = args
                .iter()
                .find(|x| !x.starts_with('-'))
                .map(String::as_str);
            if matches!(
                sub,
                Some(
                    "push"
                        | "reset"
                        | "clean"
                        | "checkout"
                        | "restore"
                        | "rebase"
                        | "merge"
                        | "commit"
                        | "rm"
                        | "mv"
                        | "stash"
                        | "tag"
                        | "branch"
                        | "pull"
                        | "fetch"
                        | "clone"
                        | "am"
                        | "apply"
                        | "gc"
                        | "prune"
                        | "filter-branch"
                        | "submodule"
                        | "worktree"
                        | "config"
                        | "remote"
                        | "init"
                        | "add"
                        | "cherry-pick"
                        | "revert"
                        | "switch"
                        | "bisect"
                )
            ) && !(sub == Some("branch") && args.len() == 1)
                && !(sub == Some("remote") && args.len() == 1)
                && !(sub == Some("stash") && args.get(1).is_some_and(|x| x == "list"))
            {
                a.flag(
                    Risk::Medium,
                    format!(
                        "`git {}` changes the repository or fetches data",
                        sub.unwrap_or("")
                    ),
                );
            }
        }
        "man" if has(&|x| x.starts_with('-') && x != "-k" && x != "-f" || x.contains('/')) => {
            a.flag(Risk::High, "`man` option or path can run programs")
        }
        "pacman" | "yay" | "paru" => {
            let first = args.first().map(String::as_str).unwrap_or("");
            let read_only = first.starts_with("-Q")
                || matches!(
                    first,
                    "-Ss"
                        | "-Si"
                        | "-Sl"
                        | "-Sg"
                        | "-F"
                        | "-Fl"
                        | "-Fy"
                        | "--version"
                        | "-V"
                        | "-h"
                );
            if !read_only {
                a.flag(
                    Risk::High,
                    format!("`{prog}` may install, remove or upgrade packages"),
                );
            }
        }
        p if PACKAGE_MANAGERS.contains(&p) => {
            if has(&|x| PACKAGE_MUTATIONS.contains(&x)) {
                a.flag(
                    Risk::High,
                    format!("`{p}` may install, remove or run packages"),
                );
            }
        }
        "ln" if has(&|x| x.starts_with('-') && x.contains('f')) => {
            a.flag(Risk::High, "`ln -f` overwrites files")
        }
        "tar" => {
            let writes = has(&|x| {
                (x.starts_with('-')
                    && !x.starts_with("--")
                    && x.chars().any(|c| "xcrud".contains(c)))
                    || matches!(
                        x,
                        "--extract" | "--create" | "--append" | "--update" | "--delete" | "--get"
                    )
            }) || args
                .first()
                .is_some_and(|x| !x.starts_with('-') && x.chars().any(|c| "xcrud".contains(c)));
            if writes {
                a.flag(Risk::Medium, "`tar` creates or changes files");
            }
        }
        "tee" | "touch" | "mkdir" | "mkfifo" | "mknod" | "patch" | "ex" | "vi" | "vim" | "nvim"
        | "nano" | "emacs" | "ed" | "sponge" | "split" | "csplit" | "rename" | "mmv" | "unzip"
        | "zip" | "gzip" | "gunzip" | "bzip2" | "xz" | "7z" | "cpio" | "ar" => {
            a.flag(Risk::Medium, format!("`{prog}` creates or changes files"));
        }
        _ => {}
    }
    // Redirect-free file readers are fine, but credentials are not.
    let mut words: Vec<String> = argv.to_vec();
    words.extend(
        argv.iter()
            .filter_map(|w| w.split_once('=').map(|(_, v)| v.to_string())),
    );
    sensitive_paths(&words, a);
}
