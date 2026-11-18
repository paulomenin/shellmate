//! What the two HTTP plugins (`provider-openai-compatible` and `decider-systemone`) have in common. It needs no
//! HTTP client, so it adds no dependency and each plugin keeps its own cargo feature.

use shellmate_api::ProviderErrorKind;

/// The error class of an HTTP status, which decides whether the next provider of a chain is tried.
pub(crate) fn classify_status(status: u16) -> ProviderErrorKind {
    match status {
        401 | 403 => ProviderErrorKind::Auth,
        408 => ProviderErrorKind::Timeout,
        429 => ProviderErrorKind::RateLimited,
        400 | 404 | 413 | 422 => ProviderErrorKind::BadRequest,
        500..=599 => ProviderErrorKind::Unavailable,
        _ => ProviderErrorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderErrorKind::*;

    #[test]
    fn statuses_map_to_the_class_that_decides_fallback() {
        for (status, kind) in [
            (401, Auth),
            (403, Auth),
            (408, Timeout),
            (429, RateLimited),
            (400, BadRequest),
            (404, BadRequest),
            (413, BadRequest),
            (422, BadRequest),
            (500, Unavailable),
            (503, Unavailable),
            (599, Unavailable),
            (200, Other),
            (302, Other),
            (418, Other),
            (600, Other),
        ] {
            assert_eq!(classify_status(status), kind, "{status}");
        }
    }
}
