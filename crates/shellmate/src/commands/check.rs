//! `check`: what shellmate's checks say about a command line, without running it.

/// `shellmate check`: the deterministic command checks, on a line you give. Nothing is run.
use shellmate_core::present::Tone;

pub(crate) fn run(shell: Option<&str>, json: bool, line: &str) -> i32 {
    use shellmate_core::command::{classify, Risk};
    use shellmate_core::validate;
    let shell = shell.map(str::to_string).unwrap_or_else(|| {
        std::env::var("SHELL")
            .ok()
            .and_then(|s| s.rsplit('/').next().map(str::to_string))
            .unwrap_or_else(|| "sh".into())
    });
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let syntax = rt.block_on(validate::syntax_error(
        &shell,
        line,
        &std::env::var("PATH").unwrap_or_default(),
    ));
    let path = std::env::var("PATH").unwrap_or_default();
    let missing = if syntax.is_none() {
        validate::missing_programs(line, &path)
    } else {
        Vec::new()
    };
    let a = classify(line);
    let reasons: Vec<String> = a
        .reasons
        .iter()
        .filter(|r| !r.starts_with("not a simple command"))
        .cloned()
        .collect();
    let risk = if reasons.is_empty() {
        Risk::Low
    } else {
        a.risk
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "shell": shell,
                "parses": syntax.is_none(),
                "syntax_error": syntax,
                "missing": missing,
                "risk": risk.label(),
                "reasons": reasons,
            })
        );
        return 0;
    }
    let out = super::ui::stdout();
    let mut clean = true;
    if let Some(e) = &syntax {
        print!(
            "{}",
            out.status(Tone::Error, &format!("does not parse in {shell}: {e}"))
        );
        clean = false;
    }
    for m in &missing {
        print!(
            "{}",
            out.status(
                Tone::Warning,
                &format!("`{m}` was not found in PATH (it may be an alias or function)")
            )
        );
        clean = false;
    }
    if risk >= Risk::Medium {
        let tone = if risk >= Risk::High {
            Tone::Error
        } else {
            Tone::Warning
        };
        print!(
            "{}",
            out.status(
                tone,
                &format!("{} risk: {}", risk.label(), reasons.join("; "))
            )
        );
        clean = false;
    }
    if clean {
        print!("{}", out.status(Tone::Success, "no findings"));
    }
    if clean {
        0
    } else {
        1
    }
}
