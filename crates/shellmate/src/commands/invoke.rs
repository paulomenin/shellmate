//! `invoke`: the request path used by the shell glue.

use crate::commands::{report, ui, EXIT_ERROR, EXIT_UNCHANGED};
use crate::{approval, commands, report_fmt, timing, tty};
use shellmate_api::Error;
use shellmate_core::engine::{Emit, Outcome};
use shellmate_core::present::{self, Presenter, Tone};
use shellmate_core::{Config, Engine, Envelope};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) fn run(
    config: Option<&std::path::Path>,
    dry_run: bool,
    explain: bool,
    timings: Option<&str>,
) -> i32 {
    let tty = tty::Tty::open();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = match rt.block_on(run_invoke(&tty, config, dry_run, explain, timings)) {
        Ok(code) => code,
        Err(e) => {
            report(&tty, &e);
            EXIT_ERROR
        }
    };
    // Security measure: whatever was typed while shellmate was working is thrown away, whatever the outcome
    // (answer, explanation, error, cancel). Nothing the user pressed without seeing the result may reach the
    // shell, and an Enter in particular would run a line they have not reviewed.
    if tty.discard_pending_input() {
        tty.write(&present::presenter(ui::tty_caps(&tty)).status(
            Tone::Warning,
            "(Enter pressed while waiting was ignored; review the result, then press Enter)",
        ));
    }
    code
}

pub(crate) async fn run_invoke(
    tty: &tty::Tty,
    config: Option<&std::path::Path>,
    dry_run: bool,
    explain: bool,
    timings: Option<&str>,
) -> Result<i32, Error> {
    let env = read_envelope()?;
    if env.action == "capture" {
        return commands::capture::run(tty, &env);
    }
    let cfg = Config::load(config)?;
    ui::use_config(&cfg);
    let mut engine = Engine::from_config(&cfg, &shellmate_builtin::registry())?;
    let startup_us = timing::since_start_us();
    if tty.is_tty() {
        // Without a terminal nobody can be asked, and every model-driven command is denied.
        engine = engine.with_approver(Arc::new(approval::TtyApprover::new(ui::tty_caps(tty))));
    }
    if dry_run {
        return show_payloads(tty, &engine, &env).await;
    }

    let streaming = cfg.ui.stream;
    // The core does not stream the model's words for `generate` (command-only), so the final text, if any
    // (a clarifying question), is printed at the end.
    let text_streamed = streaming && env.action != "generate";
    let shown = Shown::new(present::presenter(ui::tty_caps(tty)));
    let mut on_emit = |emit: Emit<'_>| shown.emit(tty, streaming, emit);
    let work = engine.run_streaming(&env, &mut on_emit);
    let Some(result) = wait(tty, &shown, work).await else {
        // dropping the work dropped the response stream, which closes the connection and stops generation
        tty.write(&shown.finish());
        tty.write(&shown.status(Tone::Muted, "[cancelled]"));
        return Ok(EXIT_UNCHANGED);
    };
    let outcome = result?;
    show_result(tty, &shown, &outcome, text_streamed);
    if explain {
        tty.write(&format!(
            "\n{}",
            report_fmt::explain_text(&outcome.report, tty.width())
        ));
    }
    if let Some(format) = timings {
        let t = report_fmt::ClientTimings {
            startup_us,
            first_output_us: shown.first_output_us(),
            process_us: timing::since_start_us(),
        };
        tty.write(&report_fmt::timings_text(
            &outcome.report,
            &t,
            format == "json",
        ));
    }
    hand_over(outcome.buffer)
}

fn read_envelope() -> Result<Envelope, Error> {
    let mut raw = Vec::new();
    std::io::stdin()
        .lock()
        .read_to_end(&mut raw)
        .map_err(|e| Error::new("glue.protocol", format!("reading stdin: {e}")))?;
    Envelope::parse(&raw)
}

/// `--dry-run`: the real wire payload of every provider in the chain, after redaction. Nothing is sent.
async fn show_payloads(tty: &tty::Tty, engine: &Engine, env: &Envelope) -> Result<i32, Error> {
    let bullet = present::presenter(ui::tty_caps(tty)).bullet();
    for p in engine.preview(env).await? {
        let wire = serde_json::to_string_pretty(&p.request).unwrap_or_default();
        let placeholders = wire.matches("<SECRET:").count();
        tty.write(&format!(
            "{bullet} provider `{}` ({:?}, context level {}): {} placeholder(s) in the payload\n{wire}\n",
            p.provider, p.trust, p.level, placeholders
        ));
    }
    Ok(EXIT_UNCHANGED)
}

/// What has reached the terminal so far, and when the first thing did (for `--timings`).
struct Shown {
    presenter: Mutex<Box<dyn Presenter>>,
    first_output: Mutex<Option<u64>>,
}

impl Shown {
    fn new(presenter: Box<dyn Presenter>) -> Self {
        Self {
            presenter: Mutex::new(presenter),
            first_output: Mutex::new(None),
        }
    }

    fn presenter(&self) -> std::sync::MutexGuard<'_, Box<dyn Presenter>> {
        self.presenter.lock().expect("presenter lock")
    }

    fn wrote_any(&self) -> bool {
        self.presenter().wrote_any()
    }

    fn first_output_us(&self) -> Option<u64> {
        *self.first_output.lock().expect("first output lock")
    }

    fn mark_first_output(&self) {
        self.first_output
            .lock()
            .expect("first output lock")
            .get_or_insert_with(timing::since_start_us);
    }

    /// What the engine shows while it works. Notices (a tool run, a refusal) are always shown; the model's
    /// words only when the person streams answers.
    fn emit(&self, tty: &tty::Tty, streaming: bool, emit: Emit<'_>) {
        let out = match emit {
            Emit::Text(_) if !streaming => return,
            Emit::Text(t) => self.presenter().answer(t),
            Emit::Notice(n) => self.presenter().notice(n),
        };
        if !out.is_empty() {
            self.mark_first_output();
        }
        tty.write(&out);
    }

    /// Flush what the presenter still holds back.
    fn finish(&self) -> String {
        self.presenter().finish()
    }

    fn status(&self, tone: Tone, text: &str) -> String {
        self.presenter().status(tone, text)
    }
}

/// Wait for `work`, drawing a spinner if nothing has appeared after a moment. `None` when the person
/// pressed Ctrl-C: `work` is dropped, whatever it was doing stops.
async fn wait<T>(
    tty: &tty::Tty,
    shown: &Shown,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::pin!(work);
    let mut ctrl_c = Box::pin(tokio::signal::ctrl_c());
    let started = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(80));
    let (mut frame, mut spinner_drawn) = (0usize, false);
    let result = loop {
        tokio::select! {
            r = &mut work => break Some(r),
            _ = &mut ctrl_c => break None,
            _ = tick.tick() => {
                if tty.is_tty() && !shown.wrote_any() && started.elapsed() >= Duration::from_millis(300) {
                    tty.write(&shown.presenter().spinner(frame));
                    frame += 1;
                    spinner_drawn = true;
                }
            }
        }
    };
    if spinner_drawn && !shown.wrote_any() {
        tty.write(&shown.presenter().clear_spinner());
    }
    result
}

/// The end of the answer: text that was not streamed, what the presenter held back, and the notes.
fn show_result(tty: &tty::Tty, shown: &Shown, outcome: &Outcome, text_streamed: bool) {
    let mut p = shown.presenter();
    if !text_streamed && !outcome.text.is_empty() {
        let out = p.answer(&outcome.text);
        shown.mark_first_output();
        tty.write(&out);
    }
    tty.write(&p.finish());
    tty.write(&p.remarks(&outcome.notes));
}

/// Give the glue the new input line on stdout (exit 0), or tell it to leave the line as it is.
fn hand_over(buffer: Option<String>) -> Result<i32, Error> {
    match buffer {
        Some(buffer) => {
            let mut out = std::io::stdout().lock();
            out.write_all(buffer.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| Error::new("glue.protocol", format!("writing stdout: {e}")))?;
            Ok(0)
        }
        None => Ok(EXIT_UNCHANGED),
    }
}
