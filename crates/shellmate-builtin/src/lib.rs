//! The first-party plugins, in one crate for now. Each one is a module behind a cargo feature of the
//! same name, and depends only on `shellmate-api`, as an external plugin would.

use shellmate_api::Registry;

#[cfg(feature = "decider-systemone")]
pub mod decider_systemone;
#[cfg(any(feature = "provider-openai-compatible", feature = "decider-systemone"))]
mod http;
#[cfg(feature = "middleware-redact")]
pub mod middleware_redact;
#[cfg(feature = "provider-openai-compatible")]
pub mod provider_openai_compatible;
#[cfg(feature = "store-tmpfs")]
pub mod store_tmpfs;
#[cfg(feature = "tool-exec")]
pub mod tool_exec;
#[cfg(feature = "tool-help")]
pub mod tool_help;
#[cfg(feature = "tool-man")]
pub mod tool_man;
#[cfg(feature = "tool-read")]
pub mod tool_read;

pub fn registry() -> Registry {
    #[allow(unused_mut)]
    let mut r = Registry::new();
    #[cfg(feature = "provider-openai-compatible")]
    provider_openai_compatible::register(&mut r);
    #[cfg(feature = "decider-systemone")]
    decider_systemone::register(&mut r);
    #[cfg(feature = "middleware-redact")]
    middleware_redact::register(&mut r);
    #[cfg(feature = "store-tmpfs")]
    store_tmpfs::register(&mut r);
    #[cfg(feature = "tool-man")]
    tool_man::register(&mut r);
    #[cfg(feature = "tool-help")]
    tool_help::register(&mut r);
    #[cfg(feature = "tool-exec")]
    tool_exec::register(&mut r);
    #[cfg(feature = "tool-read")]
    tool_read::register(&mut r);
    r
}
