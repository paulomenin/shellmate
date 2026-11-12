//! `config check`: load the config and build everything it names, without sending anything.

use crate::commands::EXIT_ERROR;
use shellmate_core::present::Tone;
use shellmate_core::{Config, Engine};

pub(crate) fn run(config: Option<&std::path::Path>) -> i32 {
    let registry = shellmate_builtin::registry();
    let result = Config::load(config).and_then(|cfg| {
        cfg.validate_with(&registry, &shellmate_core::ports::Host::system())?;
        let engine = Engine::from_config(&cfg, &registry)?; // also enforces the privacy rules (remote providers need redaction)
        let chain = cfg
            .provider_chain()
            .map(|c| c.into_iter().map(String::from).collect::<Vec<_>>())?;
        let mut risky = engine
            .risky_tool_rules()
            .into_iter()
            .map(|(tool, r, why)| (format!("tools.{tool}.preapproved \"{r}\""), why))
            .collect::<Vec<_>>();
        let env = shellmate_core::ports::SystemEnv;
        risky.extend(cfg.cmd_secrets().into_iter().filter_map(|(path, line)| {
            shellmate_core::secret_cmd::has_embedded_secret(&line, &env).then(|| {
                (
                    path,
                    "a `cmd:` argument looks like a key written in place; keep the key in a password manager or a file and name where it is"
                        .to_string(),
                )
            })
        }));
        Ok((chain, risky))
    });
    match result {
        Ok((chain, risky)) => {
            let out = super::ui::stdout();
            println!(
                "{} provider chain: {}",
                out.label(Tone::Success, "config ok;"),
                chain.join(" -> ")
            );
            for (rule, why) in risky {
                println!("{} {rule}: {why}", out.label(Tone::Warning, "warning:"));
            }
            0
        }
        Err(e) => {
            eprintln!("{}: {}", e.code, e.message);
            if let Some(h) = e.hint {
                eprintln!("  {h}");
            }
            EXIT_ERROR
        }
    }
}
