//! Secrets fetched on first use (`cmd:`): not at start-up, not for a provider that is never called, and a
//! failure is a configuration error that does not fall back.

use super::*;
use shellmate_api::Registry;
use std::path::PathBuf;

fn marker(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("sm-eng-{name}-{}", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

/// A registry with one provider plugin, `fake`, that records the settings it is built with.
fn registry(built: Arc<Mutex<Vec<serde_json::Value>>>, reply: &'static str) -> Registry {
    let mut r = Registry::new();
    r.register_provider(
        "fake",
        Box::new(move |settings| {
            built.lock().unwrap().push(settings.clone());
            Ok(FakeProvider::with(vec![Ok(reply.into()), Ok(reply.into())]) as Arc<dyn Provider>)
        }),
    );
    r
}

fn provider(name: &str, line: &str) -> String {
    format!(
        "[providers.{name}]\nplugin = \"builtin:fake\"\ntrust = \"local\"\n[providers.{name}.settings]\napi_key = \"{line}\"\nmodel = \"{name}\"\n"
    )
}

fn config(chain: &[&str], providers: &str) -> Config {
    let chain = chain
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    Config::parse(&format!(
        "version = 1\n[llm]\nchain = [{chain}]\n[conversation]\nstore = \"none\"\n[audit]\nenabled = false\n[pipeline]\nllm = []\n{providers}"
    ))
    .unwrap()
}

#[tokio::test]
async fn the_program_runs_at_the_first_request_and_not_before() {
    let m = marker("first");
    let built = Arc::default();
    let cfg = config(
        &["p"],
        &provider(
            "p",
            &format!("cmd:/bin/sh -c 'echo sk-real; touch {}'", m.display()),
        ),
    );
    let engine =
        Engine::from_config_in(&cfg, &registry(Arc::clone(&built), "hello"), Host::system())
            .unwrap();

    assert!(!m.exists(), "building the engine must not run the program");
    {
        let b = built.lock().unwrap();
        assert_eq!(b.len(), 1);
        assert!(!b[0].to_string().contains("sk-real"));
    }
    assert!(
        engine.providers[0].provider.info().model == "fake-1",
        "info() answers without the secret"
    );

    let out = engine.run(&env("explain", "ls")).await.unwrap();
    assert_eq!(out.text.trim(), "hello");
    assert!(m.exists());
    {
        let b = built.lock().unwrap();
        assert_eq!(b.len(), 2, "built again with the secret");
        assert_eq!(b[1]["api_key"], "sk-real");
    }

    // a second request reuses it: no second program run, no second build
    std::fs::remove_file(&m).unwrap();
    engine.run(&env("explain", "ls")).await.unwrap();
    assert!(!m.exists());
    assert_eq!(built.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_provider_that_is_never_called_never_runs_its_program() {
    let (m1, m2) = (marker("primary"), marker("spare"));
    let built = Arc::default();
    let providers = provider(
        "a",
        &format!("cmd:/bin/sh -c 'echo k1; touch {}'", m1.display()),
    ) + &provider(
        "b",
        &format!("cmd:/bin/sh -c 'echo k2; touch {}'", m2.display()),
    );
    let cfg = config(&["a", "b"], &providers);
    let engine = Engine::from_config_in(&cfg, &registry(built, "hi"), Host::system()).unwrap();
    engine.run(&env("explain", "ls")).await.unwrap();
    assert!(m1.exists());
    assert!(
        !m2.exists(),
        "the fallback was not needed, so its secret was not fetched"
    );
    let _ = std::fs::remove_file(&m1);
}

#[tokio::test]
async fn a_failing_program_is_a_configuration_error_and_does_not_fall_back() {
    let spare = marker("no-fallback");
    let built = Arc::default();
    let providers = provider("a", "cmd:/bin/sh -c 'echo broken >&2; exit 4'")
        + &provider(
            "b",
            &format!("cmd:/bin/sh -c 'echo k2; touch {}'", spare.display()),
        );
    let cfg = config(&["a", "b"], &providers);
    let engine = Engine::from_config_in(&cfg, &registry(built, "hi"), Host::system()).unwrap();
    let e = engine.run(&env("explain", "ls")).await.unwrap_err();
    assert_eq!(e.code, "config.secret_cmd_failed");
    assert!(
        e.message.contains("providers.a.settings.api_key"),
        "{}",
        e.message
    );
    assert!(e.message.contains("exit 4"));
    assert!(
        !spare.exists(),
        "a broken secret is not a reason to try the next provider"
    );
}

#[tokio::test]
async fn a_provider_without_cmd_secrets_is_built_once_at_start() {
    let built = Arc::default();
    let cfg = config(&["p"], &provider("p", "env:SM_ENGINE_TEST_KEY"));
    let host = Host::system().with_env(MapEnv::new().with("SM_ENGINE_TEST_KEY", "from-env"));
    let engine = Engine::from_config_in(&cfg, &registry(Arc::clone(&built), "hi"), host).unwrap();
    engine.run(&env("explain", "ls")).await.unwrap();
    let b = built.lock().unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0]["api_key"], "from-env");
}

#[tokio::test]
async fn a_dry_run_does_not_fetch_secrets() {
    let m = marker("dry");
    let built = Arc::default();
    let cfg = config(
        &["p"],
        &provider(
            "p",
            &format!("cmd:/bin/sh -c 'echo k; touch {}'", m.display()),
        ),
    );
    let engine = Engine::from_config_in(&cfg, &registry(built, "hi"), Host::system()).unwrap();
    engine.preview(&env("ask", "list files")).await.unwrap();
    assert!(!m.exists());
}

#[test]
fn a_bad_cmd_line_stops_the_engine_before_any_request() {
    let built = Arc::default();
    let cfg = config(&["p"], &provider("p", "cmd:/no/such/program"));
    let e = Engine::from_config_in(&cfg, &registry(built, "hi"), Host::system())
        .err()
        .unwrap();
    assert_eq!(e.code, "config.secret_cmd_missing");
}
