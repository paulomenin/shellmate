//! Prompt assembly (`providers-prompts.md` §4): base rules + action addendum + shell dialect notes.

use crate::action::Action;
use crate::envelope::Envelope;
use crate::response::{FENCE_LABEL, OTHER_OPTIONS_LABEL};
use shellmate_api::{Error, Message, Result};

/// What we know about the user's environment, already reduced to what may be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub shell: String,
    pub shell_version: Option<String>,
    pub os: String,
    pub cwd: Option<String>,
    pub buffer: String,
    /// Level 2: exit status of the last command, and the most recent commands (oldest first).
    pub last_status: Option<i32>,
    pub history: Vec<(i32, String)>,
    /// Level 2: the rendered section for the output captured with `shellmate run` (see `capture`).
    pub capture: Option<String>,
    /// Level 1: what kind of project the working directory is (see `collect::project`).
    pub project: Option<String>,
    /// Level 2: the git repository's state (see `collect::git_info`).
    pub git: Option<String>,
    /// The git section carries text written by others (commit subjects).
    pub git_untrusted: bool,
}

impl Context {
    pub fn from_envelope(
        env: &Envelope,
        os: String,
        home: Option<&str>,
        history_lines: usize,
    ) -> Self {
        let skip = env.hist.len().saturating_sub(history_lines);
        Self {
            shell: env.shell.clone(),
            shell_version: env.shell_version.clone(),
            os,
            cwd: env.cwd.as_deref().map(|c| tilde(c, home)),
            buffer: env.buffer.clone(),
            last_status: env.status,
            history: env.hist.iter().skip(skip).cloned().collect(),
            capture: None,
            project: None,
            git: None,
            git_untrusted: false,
        }
    }
}

/// Replace the home directory prefix with `~` so the username is not sent needlessly.
fn tilde(path: &str, home: Option<&str>) -> String {
    match home {
        Some(h) if !h.is_empty() && h != "/" => match path.strip_prefix(h) {
            Some("") => "~".to_string(),
            Some(rest) if rest.starts_with('/') => format!("~{rest}"),
            _ => path.to_string(),
        },
        _ => path.to_string(),
    }
}

pub fn detect_os(fs: &dyn crate::ports::Fs) -> String {
    fs.read_to_string(std::path::Path::new("/etc/os-release"))
        .ok()
        .and_then(|t| {
            t.lines().find_map(|l| {
                l.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_string())
            })
        })
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn dialect_notes(shell: &str) -> &'static str {
    match shell {
        "fish" => "fish syntax: no heredocs; set variables with `set`; command substitution is `(cmd)`; use `$status`, not `$?`; `&&` and `||` work in fish 3+.",
        "zsh" => "zsh syntax: POSIX-like, with zsh globbing and expansion rules (arrays are 1-indexed).",
        "bash" => "bash syntax: POSIX-like with bash extensions.",
        _ => "POSIX shell syntax.",
    }
}

fn base_rules(ctx: &Context, level: u8, extras: &Extras) -> String {
    let mut s = String::from(
        "You are shellmate, a command-line assistant that lives in the user's terminal. \
         Be concise: the answer is read in a terminal, so use plain text, short paragraphs and no tables.\n",
    );
    if level >= 1 {
        s.push_str(&format!("The user's shell is {}", ctx.shell));
        if let Some(v) = &ctx.shell_version {
            s.push_str(&format!(" {v}"));
        }
        s.push_str(&format!(" on {}.", ctx.os));
        if let Some(cwd) = &ctx.cwd {
            s.push_str(&format!(" The working directory is {cwd}."));
        }
        s.push('\n');
        s.push_str(dialect_notes(&ctx.shell));
        s.push('\n');
        if let Some(project) = &ctx.project {
            s.push_str(project);
        }
    }
    if level >= 2 {
        if let Some(st) = ctx.last_status {
            s.push_str(&format!(
                "The last command the user ran exited with status {st}.\n"
            ));
        }
        if !ctx.history.is_empty() {
            s.push_str("Recent commands, oldest first (exit status, command):\n");
            for (st, cmd) in &ctx.history {
                s.push_str(&format!("  {st} {}\n", cmd.replace('\n', " ")));
            }
        }
        if let Some(git) = &ctx.git {
            s.push_str(git);
        }
        if let Some(section) = &ctx.capture {
            s.push_str(section);
        }
    }
    s.push_str(&format!(
        "When you suggest a command, put exactly one command (it may span lines) in its own fenced block labeled `{FENCE_LABEL}`, like:\n\
         ```{FENCE_LABEL}\nls -la\n```\n\
         Steps that belong together go in that one block: join them with `&&` when a step needs the one before it to \
         succeed (`mkdir -p out && cd out`), or put them on separate lines when they are independent. Use a second block \
         only for a real alternative to the first; the user sees extra blocks as \"{OTHER_OPTIONS_LABEL}\", never as next steps.\n\
         Never put anything else in such a block. Never use placeholders that the user must edit unless unavoidable; \
         prefer commands that are safe to run. Never include secrets or API keys.\n"
    ));
    // each tool brings its own instructions (see `Tool::offer`)
    for offer in extras.tool_offers {
        s.push_str(offer);
    }
    if extras.omitted > 0 {
        s.push_str(&format!(
            "Note: {} earlier turn(s) of this conversation are not included.\n",
            extras.omitted
        ));
    }
    s
}

/// Everything beyond the current request that shapes the prompt.
#[derive(Default)]
pub struct Extras<'a> {
    /// Earlier turns of the conversation, already filtered for this provider.
    pub history: &'a [Message],
    /// The instructions of each tool offered to the model, in order (each ends with a newline).
    pub tool_offers: &'a [String],
    /// Earlier turns left out for this provider (shown to the model as a note).
    pub omitted: usize,
    /// The input line is exactly the command the previous turn suggested.
    pub unchanged_suggestion: bool,
}

fn action_addendum(action: Action) -> &'static str {
    match action {
        Action::Ask => "Answer the user's question. If a command helps, suggest the best one.",
        Action::Generate => {
            "Turn the user's request into exactly one shell command. Reply with that command in one command block and nothing else: \
             no explanation, no notes, no alternatives. If the request cannot be done with one command, or is ambiguous, \
             reply with one short sentence saying why or asking what is missing, and no command block."
        }
        Action::NewConversation => "",
        Action::Explain => {
            "Explain the command the user wrote: what it does, what each part means, and any risk (data loss, network, privileges). \
             Do not suggest a replacement unless the command is clearly wrong or unsafe."
        }
    }
}

/// Build the messages for `action` with the context allowed at `level` (0 = the user's text only).
pub fn build(action: Action, ctx: &Context, level: u8) -> Result<Vec<Message>> {
    build_with(action, ctx, level, &Extras::default())
}

/// `system`, then the conversation history, then the current request. The context (environment,
/// recent commands) belongs to the system message and is rebuilt every turn, never stored.
pub fn build_with(
    action: Action,
    ctx: &Context,
    level: u8,
    extras: &Extras,
) -> Result<Vec<Message>> {
    if ctx.buffer.trim().is_empty() {
        return Err(
            Error::new("input.empty", "the command line is empty").with_hint(match action {
                Action::Explain => "type or paste a command first, then press the shortcut",
                _ => "type your question or request first, then press the shortcut",
            }),
        );
    }
    let system = format!(
        "{}{}",
        base_rules(ctx, level, extras),
        action_addendum(action)
    );
    let mut user = match action {
        Action::Explain => format!("Explain this command:\n{}", ctx.buffer),
        _ => ctx.buffer.clone(),
    };
    if extras.unchanged_suggestion && action != Action::Explain {
        user.push_str("\n(this line is the command you suggested last, unchanged)");
    }
    let mut messages = vec![Message::system(system)];
    messages.extend_from_slice(extras.history);
    messages.push(Message::user(user));
    Ok(messages)
}

#[cfg(test)]
mod tests;
