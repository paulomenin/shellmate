//! `setup`: choose a provider and shell, write the config, install the shell block and run `doctor`.

use super::doctor::{doctor, reachable, shell_name};
use super::install_shell::{confirm, install_shell};
use super::templates::{is_loopback_url, template_for};
use super::{config_cmd::write_config, config_path};
use shellmate_api::{Error, Result};
use shellmate_core::ports::SystemEnv;
use shellmate_core::present::Tone;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

fn prompt(question: &str, default: &str) -> String {
    print!("{question} [{default}] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    let t = line.trim();
    if t.is_empty() {
        default.to_string()
    } else {
        t.to_string()
    }
}

pub struct SetupArgs {
    pub yes: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub shell: Option<String>,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
}

pub fn setup(explicit: Option<&Path>, a: SetupArgs) -> Result<String> {
    let interactive = !a.yes;
    if interactive && !std::io::stdin().is_terminal() {
        return Err(
            Error::new("setup.needs_tty", "interactive setup needs a terminal")
                .with_hint("pass --yes with --provider, --model and --shell"),
        );
    }
    let shell = a
        .shell
        .clone()
        .or_else(|| shell_name(&SystemEnv))
        .unwrap_or_default();
    let shell = if ["fish", "zsh", "bash"].contains(&shell.as_str()) {
        shell
    } else if interactive {
        prompt("shell (fish, zsh, bash)", "bash")
    } else {
        return Err(
            Error::new("setup.no_shell", "cannot tell which shell to set up")
                .with_hint("pass --shell fish|zsh|bash"),
        );
    };
    let ui = crate::commands::ui::stdout();
    println!("{} {shell}", ui.label(Tone::Muted, "shell:"));

    let ollama_up = reachable("127.0.0.1:11434");
    let provider = match a.provider.clone() {
        Some(p) => p,
        None if interactive => {
            if ollama_up {
                println!(
                    "{}",
                    ui.label(Tone::Success, "found Ollama on localhost:11434")
                );
            }
            prompt(
                "provider (ollama, lmstudio, localai, openai, openrouter, custom)",
                if ollama_up { "ollama" } else { "openrouter" },
            )
        }
        None => {
            if ollama_up {
                "ollama".into()
            } else {
                return Err(
                    Error::new("setup.no_provider", "no provider chosen").with_hint(
                        "pass --provider ollama|lmstudio|localai|openai|openrouter|custom",
                    ),
                );
            }
        }
    };
    // `custom` is any OpenAI-compatible server: ask where it is (and where its key lives).
    let (base_url, key_env) = if provider == "custom" {
        let url = match a.base_url.clone() {
            Some(u) => u,
            None if interactive => prompt("base URL (e.g. http://localhost:8000/v1)", ""),
            None => String::new(),
        };
        if url.trim().is_empty() {
            return Err(Error::new("setup.bad_base_url", "a base URL is needed")
                .with_hint("pass --base-url, e.g. --base-url http://localhost:8000/v1"));
        }
        let key = match a.api_key_env.clone() {
            Some(k) => Some(k),
            None if interactive => {
                let k = prompt(
                    "environment variable that holds the API key (empty for none)",
                    "",
                );
                (!k.is_empty()).then_some(k)
            }
            None => None,
        };
        (Some(url), key)
    } else {
        (None, None)
    };
    let remote = match provider.as_str() {
        "openai" | "openrouter" => true,
        "custom" => !base_url.as_deref().is_some_and(is_loopback_url),
        _ => false,
    };
    if remote && interactive {
        println!(
            "\n{}",
            ui.label(
                Tone::Warning,
                &format!("{provider} is a remote service. By default shellmate sends it: what you type, the shell/OS name and your working directory.\nIt does not send your command history or environment, and secrets-looking values are replaced by placeholders first.\nYour API key is read from an environment variable you name; it is never written to the config.")
            )
        );
        if !confirm("continue?", false)? {
            return Ok(ui.status(Tone::Warning, "setup cancelled"));
        }
    }
    let default_model = match provider.as_str() {
        "ollama" => "qwen2.5-coder:7b",
        "lmstudio" | "localai" | "custom" => "local-model",
        "openai" => "gpt-4o-mini",
        _ => "openai/gpt-4o-mini",
    };
    let model = match a.model.clone() {
        Some(m) => m,
        None if interactive => prompt("model", default_model),
        None => default_model.to_string(),
    };

    let mut out = String::new();
    let path = config_path(explicit, &SystemEnv);
    if path.exists() {
        out.push_str(&format!(
            "{} {}\n",
            ui.label(Tone::Muted, "keeping existing"),
            path.display()
        ));
    } else {
        let text = template_for(
            &provider,
            &model,
            base_url.as_deref(),
            key_env.as_deref(),
            Some(&shell),
        )?;
        out.push_str(&write_config(explicit, text, false)?);
    }
    print!("{out}");
    out.clear();
    out.push_str(&install_shell(&shell, a.yes, false, false)?);
    let (report, _) = doctor(explicit, false);
    out.push('\n');
    out.push_str(&report);
    if remote {
        let var = match provider.as_str() {
            "openai" => key_env.clone().or(Some("OPENAI_API_KEY".into())),
            "openrouter" => Some("OPENROUTER_API_KEY".into()),
            _ => key_env.clone(),
        };
        if let Some(var) = var {
            out.push('\n');
            out.push_str(&ui.status(
                Tone::Warning,
                &format!(
                    "set your key before using it, e.g. `export {var}=...` in your shell's environment"
                ),
            ));
        }
    }
    out.push('\n');
    out.push_str(&ui.status(
        Tone::Info,
        "try it: type `find the large files here` and press alt+m",
    ));
    Ok(out)
}
