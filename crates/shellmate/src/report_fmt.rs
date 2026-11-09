//! The text of `--explain` and `--timings`.

use shellmate_core::render;

/// What only the client can measure, in microseconds since the process started.
pub(crate) struct ClientTimings {
    /// Reading the config and building the engine.
    pub(crate) startup_us: u64,
    /// The first byte shown to the user (text or a tool notice).
    pub(crate) first_output_us: Option<u64>,
    /// Everything, up to the moment the answer was complete.
    pub(crate) process_us: u64,
}

pub(crate) fn ms3(us: u64) -> f64 {
    (us as f64 / 1000.0 * 1000.0).round() / 1000.0
}

/// `--timings`: where the time went. `json` is one object on one line, for scripts (`tools/bench`), in
/// milliseconds with three decimals.
pub(crate) fn timings_text(
    r: &shellmate_core::engine::Report,
    c: &ClientTimings,
    json: bool,
) -> String {
    let st = &r.stages;
    // the engine's own overhead: everything that is neither the model, nor the context, nor the checks
    let overhead = st
        .total_us
        .saturating_sub(st.context_us + st.llm_us + st.validate_us);
    if json {
        return format!(
            "{}\n",
            serde_json::json!({"timings": {
                "startup_ms": ms3(c.startup_us),
                "context_ms": ms3(st.context_us),
                "llm_ms": ms3(st.llm_us),
                "first_token_ms": st.first_token_us.map(ms3),
                "validate_ms": ms3(st.validate_us),
                "engine_ms": ms3(st.total_us),
                "overhead_ms": ms3(overhead),
                "first_output_ms": c.first_output_us.map(ms3),
                "process_ms": ms3(c.process_us),
                "input_tokens": r.input_tokens,
                "output_tokens": r.output_tokens,
            }})
        );
    }
    let f = |us: u64| format!("{:.1} ms", us as f64 / 1000.0);
    let mut o = format!(
        "timings:  startup {} · context {} · llm {}",
        f(c.startup_us),
        f(st.context_us),
        f(st.llm_us)
    );
    if let Some(t) = st.first_token_us {
        o.push_str(&format!(" (first text {})", f(t)));
    }
    o.push_str(&format!(
        " · checks {} · overhead {} · total {}\n",
        f(st.validate_us),
        f(overhead),
        f(c.process_us)
    ));
    o
}

/// `--explain`: one line per fact, no content.
pub(crate) fn explain_text(r: &shellmate_core::engine::Report, width: usize) -> String {
    let mut o = String::new();
    let mut line = |k: &str, v: String| {
        o.push_str(&format!(
            "{}\n",
            render::wrap(&format!("{k:<10} {v}"), width)
        ))
    };
    line(
        "provider:",
        format!(
            "{} (chain: {}; trust {:?})",
            r.provider_chain.first().map(String::as_str).unwrap_or("-"),
            r.provider_chain.join(" -> "),
            r.trust.unwrap_or(shellmate_core::config::Trust::Remote)
        ),
    );
    line(
        "context:",
        format!(
            "level {} · redaction {}",
            r.level,
            if r.redaction { "on" } else { "off" }
        ),
    );
    line(
        "history:",
        match &r.conversation {
            Some(id) => format!(
                "turn {} of conversation {id} · {} earlier turn(s) omitted",
                r.turn, r.omitted_turns
            ),
            None => "none (stateless)".into(),
        },
    );
    if !r.tools.is_empty() {
        line("tools:", r.tools.join(", "));
    }
    if let (Some(i), Some(out)) = (r.input_tokens, r.output_tokens) {
        line("tokens:", format!("{i} in / {out} out"));
    }
    line(
        "timing:",
        format!(
            "llm {} ms{}",
            r.llm_ms,
            r.first_token_ms
                .map(|t| format!(" (first text {t} ms)"))
                .unwrap_or_default()
        ),
    );
    o
}
