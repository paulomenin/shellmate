//! `audit show`, `audit export` and `audit clear`.

use shellmate_api::{Error, Result};
use shellmate_core::ports::SystemEnv;
use shellmate_core::present::{Presenter, Tone};
use shellmate_core::{audit, Config};

pub fn audit_cmd(
    cfg: &Config,
    action: &str,
    last: Option<usize>,
    session: Option<&str>,
    p: &dyn Presenter,
) -> Result<String> {
    let path = audit::log_path(&cfg.audit, &SystemEnv);
    match action {
        "show" => {
            let events = audit::read(&path, session, Some(last.unwrap_or(20)));
            if events.is_empty() {
                return Ok("no audit events\n".into());
            }
            let mut o = String::new();
            for e in events {
                let mut rest = e.as_object().cloned().unwrap_or_default();
                let ts = rest.remove("ts").and_then(|v| v.as_u64()).unwrap_or(0);
                let ev = rest
                    .remove("event")
                    .and_then(|v| v.as_str().map(String::from));
                rest.remove("v");
                let fields: Vec<String> = rest
                    .iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect();
                let ev = ev.unwrap_or_default();
                let field = |name: &str| rest.get(name).and_then(|v| v.as_str()).unwrap_or("");
                let tone = if field("decision").starts_with("denied") || field("decision") == "deny"
                {
                    Tone::Warning
                } else if !matches!(field("status"), "" | "ok") {
                    Tone::Error
                } else {
                    Tone::Info
                };
                o.push_str(&format!(
                    "{} {} {}\n",
                    p.label(Tone::Muted, &ts.to_string()),
                    p.label(tone, &format!("{ev:<14}")),
                    fields.join(" ")
                ));
            }
            Ok(o)
        }
        "export" => {
            let events = audit::read(&path, session, last);
            Ok(serde_json::to_string_pretty(&events).unwrap_or_default() + "\n")
        }
        "clear" => match std::fs::remove_file(&path) {
            Ok(()) => Ok(format!(
                "{} {}\n",
                p.label(Tone::Success, "deleted"),
                path.display()
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("no audit log\n".into()),
            Err(e) => Err(Error::new(
                "audit.failed",
                format!("{}: {e}", path.display()),
            )),
        },
        _ => Err(Error::new("audit.unknown", "unknown audit command")),
    }
}
