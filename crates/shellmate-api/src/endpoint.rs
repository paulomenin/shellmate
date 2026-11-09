use crate::Error;

/// Cleartext HTTP is only accepted for loopback hosts (no prompts or keys over the network in the clear).
pub fn check_endpoint(endpoint: &str) -> Result<(), Error> {
    let rest = if endpoint.starts_with("https://") {
        return Ok(());
    } else if let Some(r) = endpoint.strip_prefix("http://") {
        r
    } else {
        return Err(Error::new(
            "config.invalid",
            format!("endpoint `{endpoint}` must start with https:// or http://"),
        ));
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(authority)
    };
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        Ok(())
    } else {
        Err(Error::new(
            "config.insecure_endpoint",
            format!("cleartext http:// endpoint for non-loopback host `{host}`"),
        )
        .with_hint("use https://, or a loopback address for local servers"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_may_use_cleartext() {
        assert!(check_endpoint("https://api.example.com/v1").is_ok());
        assert!(check_endpoint("http://localhost:11434").is_ok());
        assert!(check_endpoint("http://127.0.0.1:1/x").is_ok());
        assert!(check_endpoint("http://[::1]:80").is_ok());
        assert!(check_endpoint("http://10.0.0.5:11434").is_err());
        assert!(check_endpoint("http://localhost.evil.com").is_err());
        assert!(check_endpoint("ftp://x").is_err());
    }
}
