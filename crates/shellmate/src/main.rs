//! The `shellmate` binary: a thin client over `shellmate-core`.
//!
//! Exit codes for `invoke` (glue contract, `glue.md` §2): 0 = replace the buffer with stdout,
//! 10 = leave it unchanged, anything else = error (message already shown on the terminal).

mod admin;
mod approval;
mod cli;
mod commands;
mod convo;
mod ctxview;
mod init;
mod report_fmt;
mod run;
mod timing;
mod tty;

use cli::{Cmd, ConfigCmd};
use commands::finish_text;

fn main() {
    timing::start();
    let cli = cli::parse();
    commands::ui::init(cli.config.as_deref());
    let code = match cli.command {
        Cmd::Invoke {
            dry_run,
            explain,
            timings,
        } => commands::invoke::run(cli.config.as_deref(), dry_run, explain, timings.as_deref()),
        Cmd::Check { shell, json, line } => {
            commands::check::run(shell.as_deref(), json, &line.join(" "))
        }
        Cmd::Context { action, full, text } => {
            commands::context::run(cli.config.as_deref(), &action, full, &text.join(" "))
        }
        Cmd::Render {
            file,
            color,
            width,
            ascii,
            no_markdown,
        } => commands::render::run(file.as_deref(), color.as_deref(), width, ascii, no_markdown),
        Cmd::Run { shell, command } => run::run(shell.as_deref(), &command),
        Cmd::Setup {
            yes,
            provider,
            model,
            shell,
            base_url,
            api_key_env,
        } => finish_text(admin::setup(
            cli.config.as_deref(),
            admin::SetupArgs {
                yes,
                provider,
                model,
                shell,
                base_url,
                api_key_env,
            },
        )),
        Cmd::InstallShell {
            shell,
            yes,
            remove,
            dry_run,
        } => commands::install_shell::run(shell, yes, remove, dry_run),
        Cmd::Doctor { json } => {
            let (out, failed) = admin::doctor(cli.config.as_deref(), json);
            print!("{out}");
            i32::from(failed)
        }
        Cmd::Audit { command } => commands::audit::run(cli.config.as_deref(), command),
        Cmd::Config {
            command: ConfigCmd::Show,
        } => finish_text(admin::config_show(
            cli.config.as_deref(),
            commands::ui::stdout().as_ref(),
        )),
        Cmd::Config {
            command: ConfigCmd::Edit,
        } => match admin::config_edit(cli.config.as_deref(), &shellmate_core::ports::SystemEnv) {
            // the editor is closed: check what was saved, like `config check`
            Ok(_) => {
                let code = commands::config_check::run(cli.config.as_deref());
                if code != 0 {
                    eprintln!("  your edit is kept; run `shellmate config edit` to fix it");
                }
                code
            }
            Err(e) => finish_text(Err(e)),
        },
        Cmd::Config {
            command:
                ConfigCmd::Init {
                    provider,
                    model,
                    force,
                },
        } => finish_text(admin::config_init(
            cli.config.as_deref(),
            provider.as_deref(),
            model.as_deref(),
            force,
        )),
        Cmd::Init { shell } => commands::shell_init::run(cli.config.as_deref(), &shell),
        Cmd::Conversation { session, command } => {
            commands::conversation::run(cli.config.as_deref(), session.as_deref(), command)
        }
        Cmd::Config {
            command: ConfigCmd::Check,
        } => commands::config_check::run(cli.config.as_deref()),
    };
    std::process::exit(code);
}
