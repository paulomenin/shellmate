//! Tool calls the model may make while answering (`approval-ux.md`, `plugins.md`).
//!
//! The model asks by replying with a `shellmate-tool` block; the engine parses it, a classifier looks at
//! the parsed call, the policy decides, and only then is anything run.
//!
//! One file per concern: `call` (parsing), `policy` (`man`), `approval` (what the person is asked),
//! `exec`, `man` and `read` (the tools themselves) and `text` (cleaning what programs wrote).

mod approval;
mod call;
mod exec;
mod man;
mod policy;
mod read;
#[cfg(test)]
mod tests;
mod text;

pub use approval::*;
pub use call::*;
pub use exec::*;
pub use man::*;
pub use policy::*;
pub use read::*;
pub use text::*;
