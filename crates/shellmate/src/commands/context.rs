//! `context`: which pieces of context the model would get, and why the rest are held back.

use crate::commands::{ui, EXIT_ERROR};
use crate::{ctxview, tty};
use shellmate_api::Error;
use shellmate_core::{Config, Engine, Envelope};

/// `shellmate context`: the same context the engine would build for a shortcut pressed right here.
pub(crate) fn run(config: Option<&std::path::Path>, action: &str, full: bool, text: &str) -> i32 {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let res = rt.block_on(async {
        let cfg = Config::load(config)?;
        let engine = Engine::from_config(&cfg, &shellmate_builtin::registry())?;
        let env = Envelope {
            version: 1,
            action: action.to_string(),
            shell: std::env::var("SHELL")
                .ok()
                .and_then(|s| s.rsplit('/').next().map(str::to_string))
                .unwrap_or_else(|| "sh".into()),
            cwd: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            session: std::env::var("SHELLMATE_SESSION")
                .ok()
                .filter(|s| !s.is_empty()),
            buffer: if text.is_empty() {
                "<your question>".into()
            } else {
                text.to_string()
            },
            ..Envelope::default()
        };
        let reports = engine.context_report(&env).await?;
        let payloads = if full {
            engine.preview(&env).await?
        } else {
            Vec::new()
        };
        Ok::<_, Error>((reports, payloads))
    });
    match res {
        Ok((reports, payloads)) => {
            let utf8 = tty::Tty::open().utf8();
            print!("{}", ctxview::render(&reports, utf8, ui::stdout().as_ref()));
            println!(
                "The last exit status and recent commands are known only when a shortcut calls shellmate, so they\n\
                 show as \"none reported\" here. `shellmate invoke --dry-run` previews a real request.\n"
            );
            for p in payloads {
                println!(
                    "payload for `{}`:\n{}\n",
                    p.provider,
                    serde_json::to_string_pretty(&p.request).unwrap_or_default()
                );
            }
            0
        }
        Err(e) => {
            eprintln!("shellmate: {}: {}", e.code, e.message);
            if let Some(h) = &e.hint {
                eprintln!("  {h}");
            }
            EXIT_ERROR
        }
    }
}
