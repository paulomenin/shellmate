use super::*;
use crate::{mapping::*, middleware::*, redactor::*, rules::*};
use async_trait::async_trait;
use shellmate_api::{
    ChatRequest, ChatStream, Error, EventStream, Middleware, Next, Registry, Result, StreamEvent,
};
use shellmate_api::{Message, Provider, Terminal};
use shellmate_testing::FakeProvider;
use std::collections::VecDeque;
use std::sync::Arc;

fn r() -> Redactor {
    Redactor::new(&[]).unwrap().with_key([7; 16])
}

fn red(text: &str) -> String {
    r().redact(text, &mut Mapping::default())
}

#[test]
fn redacts_known_secret_shapes() {
    let cases = [
        "export OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwx1234",
        "curl -H 'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U'",
        "git clone https://user:hunter2pass@github.com/x/y.git",
        "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "aws configure set aws_access_key_id AKIAIOSFODNN7EXAMPLE",
        "DB_PASSWORD='correct horse battery'",
        "set -x GITHUB_TOKEN abc123def456",
        "mysql --password s3cr3tpw -h db",
        "curl -u admin:swordfish123 https://x",
        "{\"api_key\": \"zzzz-1234-yyyy\"}",
        "curl -H 'Authorization: Bearer shortone12'",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk=\n-----END OPENSSH PRIVATE KEY-----",
    ];
    let secrets = [
        "sk-abcdefghijklmnopqrstuvwx1234",
        "eyJhbGciOiJIUzI1NiJ9",
        "hunter2pass",
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "AKIAIOSFODNN7EXAMPLE",
        "correct horse battery",
        "abc123def456",
        "s3cr3tpw",
        "swordfish123",
        "zzzz-1234-yyyy",
        "shortone12",
        "b3BlbnNzaC1rZXk=",
    ];
    for (c, s) in cases.iter().zip(secrets) {
        let out = red(c);
        assert!(!out.contains(s), "`{s}` leaked in `{out}`");
        assert!(out.contains(PREFIX), "no placeholder in `{out}`");
    }
}

#[test]
fn leaves_ordinary_text_alone() {
    for t in [
        "git log --oneline -n 5",
        "commit 3f2a9c1d4e5b6a7f8091a2b3c4d5e6f708192a3b",
        "id 550e8400-e29b-41d4-a716-446655440000",
        "keyboard=us",
        "ls --color=auto",
        "export PATH=$HOME/bin:$PATH",
        "API_KEY=$MY_KEY",
        "GITHUB_TOKEN=env:GITHUB_TOKEN",
        "find the biggest files in ~/projects",
        "the token count is high",
        "PASSWORD_PROMPT=true",
    ] {
        assert_eq!(red(t), t);
    }
}

#[test]
fn placeholders_are_stable_distinct_and_reversible() {
    let r = r();
    let mut map = Mapping::default();
    let a = r.redact(
        "A_KEY=alpha-secret-1 and again A_KEY=alpha-secret-1 and B_KEY=beta-secret-2",
        &mut map,
    );
    assert_eq!(map.len(), 2, "{a}");
    let phs: Vec<_> = a.match_indices(PREFIX).collect();
    assert_eq!(phs.len(), 3);
    assert_eq!(
        r.restore(&a, &map),
        "A_KEY=alpha-secret-1 and again A_KEY=alpha-secret-1 and B_KEY=beta-secret-2"
    );
    // stable across calls with the same key
    let b = r.redact("A_KEY=alpha-secret-1", &mut Mapping::default());
    assert!(a.starts_with(&b), "{a} vs {b}");
}

#[test]
fn a_conversation_key_makes_placeholders_stable_across_redactors() {
    let key = Some([9u8; 16]);
    let a = Redactor::new(&[])
        .unwrap()
        .redact("X_KEY=value-12345", &mut Mapping::with_key(key));
    let b = Redactor::new(&[])
        .unwrap()
        .redact("curl -H 'X_KEY=value-12345'", &mut Mapping::with_key(key));
    let ph = |s: &str| s[s.find("<SECRET:").unwrap()..s.find('>').unwrap() + 1].to_string();
    assert_eq!(ph(&a), ph(&b));
}

#[test]
fn different_keys_give_different_placeholders() {
    let a = Redactor::new(&[])
        .unwrap()
        .with_key([1; 16])
        .redact("X_KEY=value-12345", &mut Mapping::default());
    let b = Redactor::new(&[])
        .unwrap()
        .with_key([2; 16])
        .redact("X_KEY=value-12345", &mut Mapping::default());
    assert_ne!(a, b);
}

#[test]
fn already_redacted_text_is_not_redacted_again() {
    let once = red("A_KEY=alpha-secret-1");
    assert_eq!(red(&once), once);
}

#[test]
fn rules_never_match_inside_placeholders() {
    // the token rule fires first; the assignment rule must not then match `SECRET:<hex>` inside the placeholder
    let r = r();
    let mut map = Mapping::default();
    let out = r.redact(
        "OPENAI_API_KEY=sk-live-0123456789abcdefghijKLMN and secret: topsecretvalue1",
        &mut map,
    );
    assert_eq!(out.matches("<SECRET:").count(), 2, "{out}");
    assert!(
        !out.contains("<SECRET:<SECRET:"),
        "nested placeholder: {out}"
    );
    assert_eq!(map.len(), 2);
    assert_eq!(
        r.restore(&out, &map),
        "OPENAI_API_KEY=sk-live-0123456789abcdefghijKLMN and secret: topsecretvalue1"
    );
}

#[test]
fn user_patterns_and_bad_regex() {
    let r = Redactor::new(&[ExtraPattern {
        name: "corp".into(),
        regex: r"CORP-(?P<secret>\d{6})".into(),
    }])
    .unwrap();
    let out = r.redact("ticket CORP-123456 here", &mut Mapping::default());
    assert!(out.starts_with("ticket CORP-<SECRET:"), "{out}");
    assert!(Redactor::new(&[ExtraPattern {
        name: "bad".into(),
        regex: "(".into()
    }])
    .is_err());
}

/// Success criterion 3 in miniature: what the provider sees never contains the secrets,
/// and what the user sees has them back.
#[tokio::test]
async fn provider_never_sees_secrets_and_response_is_restored() {
    let provider = FakeProvider::with(vec![Ok(
        "Use it like this:\n```shellmate-command\ncurl -H \"X-Key: PLACEHOLDER\" https://x\n```"
            .to_string(),
    )]);
    let mw = RedactMiddleware::new(r()).unwrap();
    struct T(Arc<FakeProvider>);
    #[async_trait]
    impl Terminal<ChatRequest, ChatStream> for T {
        async fn call(&self, req: ChatRequest) -> Result<ChatStream> {
            // echo the first placeholder the model "saw" back in its answer
            let sys = req
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<String>();
            let ph = sys
                .split(PREFIX)
                .nth(1)
                .and_then(|s| s.split('>').next())
                .map(|t| format!("{PREFIX}{t}>"))
                .unwrap_or_else(|| "none".into());
            let mut resp = self.0.chat(&req).await.map_err(Error::from)?;
            resp.text = resp.text.replace("PLACEHOLDER", &ph);
            Ok(shellmate_api::stream_of_response(resp))
        }
    }
    let layers: Vec<Arc<dyn Middleware<ChatRequest, ChatStream>>> = vec![Arc::new(mw)];
    let req = ChatRequest {
        request_id: "t".into(),
        messages: vec![
            Message::system("cwd /work"),
            Message::user("send this with API_KEY=sk-live-0123456789abcdefghij"),
        ],
        max_tokens: None,
        temperature: None,
        redaction_key: None,
    };
    let terminal = T(provider.clone());
    let resp = shellmate_api::collect(Next::new(&layers, &terminal).run(req).await.unwrap())
        .await
        .unwrap();
    let seen = provider.seen.lock().unwrap();
    let wire: String = seen[0]
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect();
    assert!(
        !wire.contains("sk-live-0123456789abcdefghij"),
        "leaked: {wire}"
    );
    assert!(wire.contains(PREFIX));
    assert!(
        resp.text.contains("sk-live-0123456789abcdefghij"),
        "not restored: {}",
        resp.text
    );
    assert!(!resp.text.contains(PREFIX));
}

fn restore_stream(map: Mapping, deltas: &[&str]) -> RestoreStream {
    let mut ev: Vec<_> = deltas
        .iter()
        .map(|d| Ok(StreamEvent::Text((*d).to_string())))
        .collect();
    ev.push(Ok(StreamEvent::Done {
        finish_reason: Some("stop".into()),
    }));
    RestoreStream {
        inner: shellmate_api::stream_from(ev),
        map,
        held: String::new(),
        pending: VecDeque::new(),
    }
}

async fn drain(mut s: RestoreStream) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(ev) = s.next().await {
        match ev.unwrap() {
            StreamEvent::Text(t) => out.push(t),
            StreamEvent::Done { .. } => out.push("<done>".into()),
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn placeholders_split_across_deltas_are_restored_without_leaking_fragments() {
    let r = r();
    let mut map = Mapping::default();
    let red = r.redact("TOKEN_KEY=very-secret-value-1", &mut map);
    let ph = &red[red.find("<SECRET:").unwrap()..];
    // feed one character at a time
    let pieces: Vec<String> = format!("use {ph} now").chars().map(String::from).collect();
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    let out = drain(restore_stream(map, &refs)).await;
    let joined: String = out.iter().filter(|s| *s != "<done>").cloned().collect();
    assert_eq!(joined, "use very-secret-value-1 now");
    assert!(
        out.iter().all(|piece| !piece.contains("<SECRET")),
        "a partial placeholder was emitted: {out:?}"
    );
    assert_eq!(out.last().unwrap(), "<done>");
}

#[tokio::test]
async fn ordinary_angle_brackets_pass_through_and_dangling_prefix_is_flushed_at_the_end() {
    let out = drain(restore_stream(
        Mapping::default(),
        &["a < b and c <", "SECR"],
    ))
    .await;
    let joined: String = out.iter().filter(|s| *s != "<done>").cloned().collect();
    assert_eq!(joined, "a < b and c <SECR");
    assert_eq!(out.last().unwrap(), "<done>");
}

#[test]
fn partial_placeholder_detection() {
    for held in ["<", "<SEC", "<SECRET:", "<SECRET:ab1"] {
        assert_eq!(partial_placeholder_len(held), held.len(), "{held}");
    }
    for free in ["abc", "a < b", "<SECRET:ab12>", "<SECRET:xyz", "<b>"] {
        assert_eq!(partial_placeholder_len(free), 0, "{free}");
    }
    assert_eq!(partial_placeholder_len("text <SE"), 3);
}

// ---- tests written from a mutation run ----

#[test]
fn every_redactor_gets_its_own_random_key() {
    let value = "API_KEY=some-long-secret-value-0001";
    let outputs: std::collections::HashSet<String> = (0..4)
        .map(|_| {
            Redactor::new(&[])
                .unwrap()
                .redact(value, &mut Mapping::default())
        })
        .collect();
    assert_eq!(
        outputs.len(),
        4,
        "placeholders repeat across redactors: {outputs:?}"
    );
    for o in &outputs {
        assert!(o.starts_with("API_KEY=<SECRET:"), "{o}");
    }
    // a fixed key, by contrast, is deterministic
    let fixed = |k: u8| r().with_key([k; 16]).redact(value, &mut Mapping::default());
    assert_eq!(fixed(1), fixed(1));
    assert_ne!(fixed(1), fixed(2));
}

#[test]
fn values_whose_short_placeholders_collide_get_longer_ones_and_both_restore() {
    // found by search for the fixed key [7; 16]: the same first six hex characters
    let (a, b) = ("secret-value-632", "secret-value-1245");
    let r = r();
    let mut map = Mapping::default();
    let one = r.redact(&format!("API_KEY={a}"), &mut map);
    let two = r.redact(&format!("API_KEY={b}"), &mut map);
    let ph = |s: &str| s[s.find("<SECRET:").unwrap()..].to_string();
    assert_ne!(ph(&one), ph(&two), "two secrets share a placeholder");
    assert!(
        ph(&one).len() < ph(&two).len(),
        "{} vs {}",
        ph(&one),
        ph(&two)
    );
    let both = format!("{one} and {two}");
    assert_eq!(map.restore(&both), format!("API_KEY={a} and API_KEY={b}"));
    assert_eq!(map.len(), 2);
}

#[test]
fn the_mapping_knows_when_it_holds_something() {
    let r = r();
    let mut map = Mapping::default();
    assert!(map.is_empty());
    assert_eq!(map.len(), 0);
    r.redact("API_KEY=some-long-secret-value-0001", &mut map);
    assert!(!map.is_empty());
    assert_eq!(map.len(), 1);
    // the same value again is not a second entry
    r.redact("API_KEY=some-long-secret-value-0001", &mut map);
    assert_eq!(map.len(), 1);
}

#[test]
fn the_rule_names_include_every_builtin_and_the_users_own() {
    let extra = ExtraPattern {
        name: "corp-id".into(),
        regex: r"CORP-(?P<secret>\d{6})".into(),
    };
    let names = Redactor::new(&[extra]).unwrap().rule_names().join(",");
    for n in [
        "private-key",
        "url-password",
        "token",
        "jwt",
        "auth-header",
        "authorization-header",
        "basic-auth-flag",
        "assignment",
        "fish-set",
        "secret-flag",
        "corp-id",
    ] {
        assert!(names.contains(n), "{n} missing from {names}");
    }
}

#[test]
fn the_middleware_is_registered_under_its_name_and_builds_from_settings() {
    let mut registry = Registry::default();
    register(&mut registry);
    let mw = registry
        .build_llm_middleware(PLUGIN_ID, &serde_json::json!({}))
        .expect("registered");
    assert_eq!(mw.id(), "builtin:redact");
    assert!(registry
        .build_llm_middleware(PLUGIN_ID, &serde_json::json!({"nope": 1}))
        .is_err());
    assert!(registry
        .build_llm_middleware("other", &serde_json::json!({}))
        .is_err());
}

#[test]
fn a_possible_placeholder_is_held_back_only_up_to_a_sane_length() {
    let hex = |n: usize| format!("<SECRET:{}", "a".repeat(n));
    let prefix = "<SECRET:".len();
    assert_eq!(partial_placeholder_len(&hex(79 - prefix)), 79);
    assert_eq!(partial_placeholder_len(&hex(80 - prefix)), 0);
    assert_eq!(partial_placeholder_len(&hex(200)), 0);
}

#[tokio::test]
async fn text_is_passed_on_as_it_arrives_and_never_as_empty_pieces() {
    let out = drain(restore_stream(Mapping::default(), &["hello ", "world"])).await;
    assert_eq!(out, ["hello ", "world", "<done>"]);
    // a delta that is entirely a possible placeholder start is held, not emitted as ""
    let out = drain(restore_stream(Mapping::default(), &["<SE", "CRET:ab", "c"])).await;
    assert!(out.iter().all(|p| !p.is_empty()), "{out:?}");
    let joined: String = out.iter().filter(|s| *s != "<done>").cloned().collect();
    assert_eq!(joined, "<SECRET:abc");
    // text before a held-back start goes out at once
    let out = drain(restore_stream(Mapping::default(), &["ab <SE"])).await;
    assert_eq!(out[0], "ab ");
}

mod streaming_properties {
    use super::*;
    use proptest::prelude::*;

    /// A fixed-key redactor (built once) and a mapping holding two secrets, with their placeholders.
    fn fresh() -> (Mapping, Vec<String>) {
        static R: std::sync::OnceLock<Redactor> = std::sync::OnceLock::new();
        let r = R.get_or_init(r);
        let mut map = Mapping::default();
        let red = r.redact(
            "A_KEY=first-secret-value-1 B_TOKEN=second-secret-value-2",
            &mut map,
        );
        let phs = placeholder_re()
            .find_iter(&red)
            .map(|m| m.as_str().to_string())
            .collect();
        (map, phs)
    }

    /// Text made of prose, angle brackets, unknown placeholders and the two real ones.
    fn text() -> impl Strategy<Value = String> {
        let atom = prop_oneof![
            "[a-z .]{0,6}",
            Just("<".to_string()),
            Just("<SEC".to_string()),
            Just("<SECRET:".to_string()),
            Just("<SECRET:zz>".to_string()),
            Just(">".to_string()),
            prop::sample::select(fresh().1),
        ];
        prop::collection::vec(atom, 0..12).prop_map(|v| v.concat())
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 500, failure_persistence: None, ..ProptestConfig::default() })]

        /// However the provider splits its deltas, the user gets the text with every real placeholder
        /// restored, and no piece ever shows half a placeholder that a later piece completes.
        #[test]
        fn restored_text_is_independent_of_chunking(
            text in text(), cuts in prop::collection::vec(1usize..7, 0..30),
        ) {
            let (map, phs) = fresh();
            prop_assert_eq!(phs.len(), 2);
            let expected = map.restore(&text);
            let chars: Vec<char> = text.chars().collect();
            let (mut parts, mut at) = (Vec::new(), 0);
            for c in cuts {
                let end = (at + c).min(chars.len());
                parts.push(chars[at..end].iter().collect::<String>());
                at = end;
            }
            parts.push(chars[at..].iter().collect::<String>());
            let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            let out = rt.block_on(drain(restore_stream(map, &refs)));
            let joined: String = out.iter().filter(|s| *s != "<done>").cloned().collect();
            prop_assert_eq!(joined, expected);
            prop_assert_eq!(out.last().unwrap(), "<done>");
        }
    }
}
