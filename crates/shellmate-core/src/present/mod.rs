//! How shellmate presents text to a person: what a message means (its tone), and how it is drawn.
//!
//! The engine says *what* happened (a command ran, a command was refused, a check found something) as
//! structured values: [`Notice`] while a request runs and [`Remark`] after the answer. A [`Presenter`] turns
//! those, the model's answer text and errors into the bytes for the terminal. Keeping the two apart means a
//! new look (colours, markdown) is a new presenter, not a change in the engine, and every presenter can be
//! tested with plain strings and no terminal.
//!
//! [`PlainPresenter`] is the text-only presenter and the fallback for every terminal that cannot show more.

mod level;
mod plain;
mod rich;
mod theme;
mod types;

pub use level::{detect_level, locale_is_utf8, ColorSetting, Level};
pub use plain::PlainPresenter;
pub use rich::RichPresenter;
pub use theme::{Role, Theme};
pub use types::{Caps, Mark, Notice, Remark, Tone};

use shellmate_api::Error;

/// Writes shellmate's output. Every method returns the bytes to write and touches no terminal.
///
/// All text that did not come from shellmate itself (the model's words, tool output, command text) is
/// escaped by the presenter, so a presenter is the only place that has to be trusted with the terminal.
pub trait Presenter: Send {
    /// A piece of the model's answer as it arrives (a whole answer works too). Words are held back until
    /// they are complete, so the result may be empty.
    fn answer(&mut self, text: &str) -> String;

    /// A tool run or refusal while the request is still running.
    fn notice(&mut self, notice: &Notice) -> String;

    /// The end of the answer: the last held-back word, and a fresh line if the cursor is not on one.
    fn finish(&mut self) -> String;

    /// The notes after the answer (other options, checks, warnings). Empty when there are none.
    fn remarks(&mut self, remarks: &[Remark]) -> String;

    /// True once any answer text or notice has been produced (the spinner must stop).
    fn wrote_any(&self) -> bool;

    /// A failure, with its hint if it has one.
    fn error(&self, error: &Error) -> String;

    /// One line about what shellmate itself did (`[cancelled]`).
    fn status(&self, tone: Tone, text: &str) -> String;

    /// The waiting indicator at animation step `step`; it starts with `\r` so it redraws in place.
    fn spinner(&self, step: usize) -> String;

    /// Erases the waiting indicator.
    fn clear_spinner(&self) -> String;

    /// The marker in front of a listed item.
    fn bullet(&self) -> &'static str;

    /// A short piece of text in the look of `tone` (a status word in a report). The text is escaped here.
    fn label(&self, tone: Tone, text: &str) -> String;

    /// A failure of a command that prints plain lines: `code: message`, and the hint on the next line.
    fn cli_error(&self, error: &Error) -> String;
}

/// The presenter for a terminal described by `caps`: plain text at the plain level, styled otherwise.
pub fn presenter(caps: Caps) -> Box<dyn Presenter> {
    match caps.level {
        Level::Plain => Box::new(PlainPresenter::new(caps)),
        _ => Box::new(RichPresenter::new(caps)),
    }
}
