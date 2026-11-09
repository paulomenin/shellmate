//! Property tests for redaction, the privacy guarantee for remote providers: a secret in any supported
//! form never survives `redact`, and the original text comes back exactly.

use proptest::prelude::*;
use shellmate_middleware_redact::{Mapping, Redactor};

/// Built once: compiling the rules for every case would dominate the run time.
fn redactor() -> &'static Redactor {
    static R: std::sync::OnceLock<Redactor> = std::sync::OnceLock::new();
    R.get_or_init(|| Redactor::new(&[]).unwrap().with_key([7; 16]))
}

/// Secret values in the shapes the built-in rules know, each wrapped in the context it appears in.
/// Returns (text containing the secret, the secret).
fn leaky() -> impl Strategy<Value = (String, String)> {
    let word = "[A-Za-z0-9]{12,28}";
    let name = prop::sample::select(vec![
        "API_KEY",
        "GITHUB_TOKEN",
        "DB_PASSWORD",
        "client_secret",
        "passwd",
        "AWS_SECRET_ACCESS_KEY",
    ]);
    prop_oneof![
        // vendor token shapes
        "[A-Za-z0-9]{36}".prop_map(|t| {
            let s = format!("ghp_{t}");
            (format!("token is {s} ok"), s)
        }),
        "[A-Z0-9]{16}".prop_map(|t| {
            let s = format!("AKIA{t}");
            (format!("aws {s}"), s)
        }),
        "[A-Za-z0-9_-]{24,40}".prop_map(|t| {
            let s = format!("sk-{t}");
            (format!("key {s}"), s)
        }),
        "[A-Za-z0-9]{40}".prop_map(|t| {
            let s = format!("github_pat_{t}");
            (format!("{s}\n"), s)
        }),
        // assignments, bare and quoted (quoted values may hold spaces and unicode)
        (name.clone(), word).prop_map(|(n, v)| (format!("export {n}={v}"), v)),
        (
            name.clone(),
            "[A-Za-z0-9 äöü✓!@#%^&*()-]{6,20}".prop_filter("not blank", |v| !v.trim().is_empty())
        )
            .prop_map(|(n, v)| (format!("{n}=\"{v}\""), v)),
        (name.clone(), word).prop_map(|(n, v)| (format!("{{\"{n}\": \"{v}\"}}"), v)),
        // command lines
        (name, word).prop_map(|(n, v)| (format!("set -x {n} {v}"), v)),
        word.prop_map(|v| (format!("curl -u admin:{v} https://x"), v)),
        word.prop_map(|v| (format!("git clone https://user:{v}@host/r.git"), v)),
        word.prop_map(|v| (format!("mysql --password {v} -h db"), v)),
        word.prop_map(|v| (format!("curl -H 'Authorization: Bearer {v}'"), v)),
        // a private key block, closed or cut off
        "[A-Za-z0-9+/]{20,60}".prop_map(|b| (
            format!("-----BEGIN RSA PRIVATE KEY-----\n{b}\n-----END RSA PRIVATE KEY-----"),
            b
        )),
        "[A-Za-z0-9+/]{20,60}".prop_map(|b| (format!("-----BEGIN PRIVATE KEY-----\n{b}"), b)),
    ]
}

/// Ordinary prose around a secret, including lines that look nearly like one.
fn around() -> impl Strategy<Value = String> {
    "[a-z .,\n]{0,40}"
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, failure_persistence: None, ..ProptestConfig::default() })]

    /// The secret never appears in what is sent, wherever it sits in the text.
    #[test]
    fn a_secret_never_survives_redaction((text, secret) in leaky(), before in around(), after in around()) {
        let input = format!("{before}\n{text}\n{after}");
        let out = redactor().redact(&input, &mut Mapping::default());
        prop_assert!(!out.contains(&secret), "leaked {secret:?} in {out:?}");
        prop_assert!(out.contains("<SECRET:"), "no placeholder in {out:?}");
    }

    /// Restoring gives back exactly the text that was redacted.
    #[test]
    fn restore_inverts_redact((text, _s) in leaky(), before in around(), after in around()) {
        let input = format!("{before}\n{text}\n{after}");
        let r = redactor();
        let mut map = Mapping::default();
        let out = r.redact(&input, &mut map);
        prop_assert_eq!(r.restore(&out, &map), input);
    }

    /// Arbitrary text, with or without secrets: restoring always gives the original back,
    /// unless the text already contained something shaped like a placeholder.
    #[test]
    fn restore_inverts_redact_for_any_text(input in "\\PC{0,200}") {
        prop_assume!(!input.contains("<SECRET:"));
        let r = redactor();
        let mut map = Mapping::default();
        let out = r.redact(&input, &mut map);
        prop_assert_eq!(r.restore(&out, &map), input);
    }

    /// Redacting twice changes nothing more than redacting once, so a retry or a second middleware
    /// pass cannot double-wrap a value.
    #[test]
    fn redaction_is_idempotent((text, _s) in leaky(), before in around()) {
        let input = format!("{before}\n{text}");
        let r = redactor();
        let mut map = Mapping::default();
        let once = r.redact(&input, &mut map);
        let twice = r.redact(&once, &mut map);
        prop_assert_eq!(once, twice);
    }

    /// The same value always gets the same placeholder in a conversation, and different values never share one.
    #[test]
    fn placeholders_are_stable_and_distinct(a in "[A-Za-z0-9]{12,28}", b in "[A-Za-z0-9]{12,28}") {
        prop_assume!(a != b);
        let r = redactor();
        let mut map = Mapping::default();
        let one = r.redact(&format!("API_KEY={a}"), &mut map);
        let again = r.redact(&format!("API_KEY={a}"), &mut map);
        let other = r.redact(&format!("API_KEY={b}"), &mut map);
        prop_assert_eq!(&one, &again);
        prop_assert_ne!(&one, &other);
    }

    /// A placeholder says nothing about the value: a different key gives a different placeholder.
    #[test]
    fn placeholders_depend_on_the_key(v in "[A-Za-z0-9]{12,28}", k1 in any::<[u8; 16]>(), k2 in any::<[u8; 16]>()) {
        prop_assume!(k1 != k2);
        let text = format!("API_KEY={v}");
        // the key lives in the mapping for a conversation, which is how the engine keeps it stable
        let a = redactor().redact(&text, &mut Mapping::with_key(Some(k1)));
        let b = redactor().redact(&text, &mut Mapping::with_key(Some(k2)));
        prop_assert_ne!(a, b);
    }

    /// References and literals are not secrets and are left alone.
    #[test]
    fn references_are_left_alone(
        name in prop::sample::select(vec!["API_KEY", "GITHUB_TOKEN", "DB_PASSWORD"]),
        value in prop::sample::select(vec!["$MY_KEY", "${KEY}", "env:KEY", "file:/run/k", "true", "false", "null"]),
    ) {
        let text = format!("{name}={value}");
        prop_assert_eq!(redactor().redact(&text, &mut Mapping::default()), text);
    }
}
