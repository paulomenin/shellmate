//! Tool calls the model may make while answering (`approval-ux.md`, `plugins.md`).
//!
//! The model asks by replying with a `shellmate-tool` block; the engine parses it, a classifier looks at
//! the parsed call, the policy decides, and only then is anything run.
//!
//! One file per concern: `call` (parsing), `approval` (what the person is asked), `exec` and `read` (what the
//! tools still run from the core) and `text` (cleaning what programs wrote).

mod approval;
mod call;
mod exec;
mod read;
#[cfg(test)]
mod tests;
mod text;

pub use approval::*;
pub use call::*;
pub use exec::*;
pub use read::*;
pub use text::*;
