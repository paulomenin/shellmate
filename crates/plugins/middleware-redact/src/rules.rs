//! The redaction rules: what counts as a secret, and which values are safe to leave alone.

use regex::Regex;

pub(crate) const PREFIX: &str = "<SECRET:";

pub(crate) struct Rule {
    pub(crate) name: String,
    pub(crate) re: Regex,
    /// Capture groups that may hold the secret, first one that matched wins; none = whole match.
    pub(crate) groups: &'static [&'static str],
}

/// Values that are never secrets: references, placeholders and common literals.
pub(crate) fn is_safe_value(v: &str) -> bool {
    let t = v.trim();
    t.is_empty()
        || t.starts_with('$')
        || t.starts_with(PREFIX)
        || t.starts_with("env:")
        || t.starts_with("file:")
        || matches!(
            t.to_ascii_lowercase().as_str(),
            "true"
                | "false"
                | "yes"
                | "no"
                | "null"
                | "none"
                | "0"
                | "1"
                | "..."
                | "bearer"
                | "basic"
                | "token"
        )
}

pub(crate) fn placeholder_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<SECRET:[0-9a-f]+>").expect("placeholder regex"))
}

pub(crate) fn builtin_rules() -> Vec<Rule> {
    let defs: &[(&str, &str, &'static [&'static str])] = &[
        (
            "private-key",
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)",
            &[],
        ),
        (
            "url-password",
            r"://[^/\s:@]+:(?P<secret>[^@\s/]+)@",
            &["secret"],
        ),
        (
            "token",
            r"\b(?:sk-ant-[A-Za-z0-9_\-]{20,}|sk-[A-Za-z0-9_\-]{20,}|gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|xox[baprs]-[A-Za-z0-9\-]{10,}|AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|AIza[0-9A-Za-z_\-]{35}|glpat-[A-Za-z0-9_\-]{20,}|npm_[A-Za-z0-9]{36}|hf_[A-Za-z0-9]{30,})",
            &[],
        ),
        (
            "jwt",
            r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
            &[],
        ),
        (
            "auth-header",
            r"(?i)\b(?:bearer|basic|token)\s+(?P<secret>[A-Za-z0-9\-._~+/]{16,}=*)",
            &["secret"],
        ),
        (
            // whatever follows `Authorization:`, however short (the rule above needs 16 characters)
            "authorization-header",
            r#"(?i)\bauthorization["']?\s*[:=]\s*(?:(?:bearer|basic|token)\s+)?(?P<secret>[^\s'",;]+)"#,
            &["secret"],
        ),
        (
            "basic-auth-flag",
            r"(?i)\s-u\s+[^\s:]+:(?P<secret>\S+)",
            &["secret"],
        ),
        (
            "assignment",
            r#"(?i)(?:[A-Za-z0-9_\-]*(?:key|token|secret|passw(?:or)?d|pwd|credentials?))s?["']?\s*[=:]\s*(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)'|(?P<bare>[^\s'",;]+))"#,
            &["dq", "sq", "bare"],
        ),
        (
            "fish-set",
            r#"(?i)\bset\s+(?:-\S+\s+)*[A-Za-z0-9_]*(?:key|token|secret|passw(?:or)?d|pwd)s?\s+(?:"(?P<dq>[^"\n]+)"|'(?P<sq>[^'\n]+)'|(?P<bare>[^\s'"]+))"#,
            &["dq", "sq", "bare"],
        ),
        (
            "secret-flag",
            r"(?i)--?[A-Za-z\-]*(?:password|passwd|token|secret|api-?key)\s+(?P<secret>[^\s\-]\S*)",
            &["secret"],
        ),
    ];
    defs.iter()
        .map(|(name, re, groups)| Rule {
            name: (*name).into(),
            re: Regex::new(re).expect("built-in redaction regex"),
            groups,
        })
        .collect()
}
