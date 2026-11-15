//! The command line: what `shellmate` accepts.

use crate::convo;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "shellmate",
    version,
    about = "A shell companion: ask, generate and explain commands from your prompt"
)]
pub(crate) struct Cli {
    /// Use this config file instead of ~/.config/shellmate/config.toml
    #[arg(long, global = true, value_name = "PATH", env = "SHELLMATE_CONFIG")]
    pub(crate) config: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Cmd,
}

/// The command groups shown in `--help`, in order. Every subcommand must be
/// listed once (a test checks it), so a new command cannot go missing.
const CATEGORIES: &[(&str, &[&str])] = &[
    (
        "Requests and context",
        &["context", "check", "run", "conversation"],
    ),
    (
        "Set up",
        &[
            "setup",
            "install-shell",
            "init",
            "config",
            "render",
            "doctor",
        ],
    ),
    ("Privacy", &["audit"]),
    ("Used by the shell integration", &["invoke"]),
];

/// The top-level command, with `Commands:` split into the groups above.
pub(crate) fn command() -> clap::Command {
    let cmd = Cli::command();
    let mut template = String::from("{about}\n\n{usage-heading} {usage}\n");
    let width = cmd
        .get_subcommands()
        .map(|c| c.get_name().len())
        .max()
        .unwrap_or(0);
    for (title, names) in CATEGORIES {
        template.push_str(&format!("\n{title}:\n"));
        for name in *names {
            if let Some(sub) = cmd.find_subcommand(name) {
                let about = sub.get_about().map(|a| a.to_string()).unwrap_or_default();
                template.push_str(&format!("  {name:<width$}  {about}\n"));
            }
        }
    }
    template.push_str(&format!(
        "\nOther:\n  {:<width$}  Print this message or the help of the given subcommand(s)\n",
        "help"
    ));
    template.push_str("\nOptions:\n{options}");
    cmd.help_template(template)
}

/// Parse the process arguments, exiting with help or an error like clap does.
pub(crate) fn parse() -> Cli {
    let matches = command().get_matches();
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

#[derive(Subcommand)]
pub(crate) enum Cmd {
    /// Run an action for the shell glue (reads the envelope on stdin)
    Invoke {
        /// Show the request that would be sent and exit without contacting any provider
        #[arg(long)]
        dry_run: bool,
        /// After the answer, print what was used: provider, context, tools, tokens, timing
        #[arg(long)]
        explain: bool,
        /// After the answer, print where the time went: `--timings` (one line) or `--timings=json`
        #[arg(
            long,
            value_name = "FORMAT",
            num_args = 0..=1,
            default_missing_value = "text",
            value_parser = ["text", "json"]
        )]
        timings: Option<String>,
    },
    /// Show what each provider would be told about your shell, piece by piece (nothing is sent)
    Context {
        /// The action to preview: ask, generate or explain
        #[arg(long, default_value = "ask")]
        action: String,
        /// Also print the exact payload after redaction
        #[arg(long)]
        full: bool,
        /// A sample of what you would type
        text: Vec<String>,
    },
    /// Check a command line without running it: does it parse, do its programs exist, how risky is it
    Check {
        /// The shell to parse it with (default: your login shell)
        #[arg(long)]
        shell: Option<String>,
        /// One JSON object instead of text; the exit status is then always 0
        #[arg(long)]
        json: bool,
        /// The command line
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        line: Vec<String>,
    },
    /// Show how text looks with your terminal's colours and markdown formatting (a preview for your settings)
    Render {
        /// A markdown file to show (default: read it from the input)
        file: Option<PathBuf>,
        /// How much styling to use (default: decided from your terminal and environment, like an answer)
        #[arg(long, value_name = "LEVEL", value_parser = ["auto", "plain", "mono", "16"])]
        color: Option<String>,
        /// Columns to wrap at (default: your terminal's width)
        #[arg(long, value_name = "COLUMNS")]
        width: Option<usize>,
        /// Use ASCII symbols instead of Unicode ones
        #[arg(long)]
        ascii: bool,
        /// Show the text as written, without formatting the markdown
        #[arg(long)]
        no_markdown: bool,
    },
    /// Run a command as usual and keep a copy of its output for your next question (level 2 context)
    Run {
        /// Run the rest as one line with this shell (fish, zsh, bash, sh): for pipes and redirects
        #[arg(long, value_name = "SHELL")]
        shell: Option<String>,
        /// The command and its arguments
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Guided first-time setup: config, shell integration, health check
    Setup {
        /// Don't ask; use the given options
        #[arg(long)]
        yes: bool,
        /// ollama, lmstudio, localai, openai, openrouter or custom
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        shell: Option<String>,
        /// For `--provider custom`: the base URL of the OpenAI-compatible server, e.g. http://localhost:8000/v1
        #[arg(long, value_name = "URL")]
        base_url: Option<String>,
        /// For `--provider custom`: the name of the environment variable that holds its API key
        #[arg(long, value_name = "NAME")]
        api_key_env: Option<String>,
    },
    /// Add (or with --remove, delete) the shellmate block in your shell's rc file
    InstallShell {
        /// fish, zsh or bash (default: your login shell)
        shell: Option<String>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        remove: bool,
        /// Show the change without applying it
        #[arg(long)]
        dry_run: bool,
    },
    /// Check the installation and explain how to fix problems
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// The audit log: what shellmate did on your behalf (never your prompts or answers)
    Audit {
        #[command(subcommand)]
        command: AuditCmd,
    },
    /// Print the shell integration for SHELL (fish, zsh or bash)
    Init { shell: String },
    /// Manage conversations (follow-ups share the shell session's conversation)
    Conversation {
        /// The shell session (set by the shell integration)
        #[arg(long, env = "SHELLMATE_SESSION")]
        session: Option<String>,
        #[command(subcommand)]
        command: convo::ConvoCmd,
    },
    /// Inspect, edit or write the configuration
    Config {
        #[command(subcommand)]
        command: ConfigCmd,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCmd {
    /// Validate the config, including provider settings and secret references (no network)
    Check,
    /// Print the effective configuration (secrets appear only as references)
    Show,
    /// Open the config file in $VISUAL or $EDITOR, then check that it still loads
    Edit,
    /// Write a documented starting config (provider examples are comments unless --provider is given)
    Init {
        /// Write an active config for this provider: openrouter, openai, ollama, lmstudio, localai or custom
        #[arg(long, requires = "model")]
        provider: Option<String>,
        /// The model to use with --provider, as the provider lists it. It is never guessed
        #[arg(long, requires = "provider")]
        model: Option<String>,
        /// Overwrite an existing file
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AuditCmd {
    /// Print recent events
    Show {
        #[arg(long)]
        last: Option<usize>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Print events as JSON
    Export {
        #[arg(long)]
        last: Option<usize>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Delete the log
    Clear,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_subcommand_is_in_exactly_one_group() {
        let cmd = Cli::command();
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let n = CATEGORIES
                .iter()
                .filter(|(_, names)| names.contains(&sub.get_name()))
                .count();
            assert_eq!(n, 1, "{} must be in exactly one help group", sub.get_name());
        }
        for (_, names) in CATEGORIES {
            for name in *names {
                assert!(
                    cmd.find_subcommand(name).is_some(),
                    "unknown command {name}"
                );
            }
        }
    }
}
