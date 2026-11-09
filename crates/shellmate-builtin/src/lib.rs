//! Registers the first-party plugins enabled by cargo features.

use shellmate_api::Registry;

pub fn registry() -> Registry {
    #[allow(unused_mut)]
    let mut r = Registry::new();
    #[cfg(feature = "provider-openai-compatible")]
    shellmate_provider_openai_compatible::register(&mut r);
    #[cfg(feature = "decider-systemone")]
    shellmate_decider_systemone::register(&mut r);
    #[cfg(feature = "middleware-redact")]
    shellmate_middleware_redact::register(&mut r);
    #[cfg(feature = "store-tmpfs")]
    shellmate_store_tmpfs::register(&mut r);
    r
}
