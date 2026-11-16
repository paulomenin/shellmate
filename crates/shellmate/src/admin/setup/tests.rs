use super::*;
use crate::admin::prompter::Scripted;
use shellmate_core::ports::MapEnv;
use shellmate_core::Config;
use std::cell::RefCell;
use std::collections::VecDeque;

type Reply = Result<Option<Vec<String>>>;

/// A machine for the tests: an environment, ports that answer, and the replies of the servers, in order.
struct Machine {
    env: MapEnv,
    up: Vec<u16>,
    replies: RefCell<VecDeque<Reply>>,
    calls: RefCell<Vec<(Kind, Backend)>>,
}

impl Machine {
    fn new(env: MapEnv, up: &[u16]) -> Self {
        Self {
            env,
            up: up.to_vec(),
            replies: RefCell::new(VecDeque::new()),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn reply(self, r: Reply) -> Self {
        self.replies.borrow_mut().push_back(r);
        self
    }

    fn run<T>(&self, f: impl FnOnce(&Services) -> T) -> T {
        let probe = |port: u16| self.up.contains(&port);
        let connect = |kind: Kind, b: &Backend| {
            self.calls.borrow_mut().push((kind, b.clone()));
            self.replies
                .borrow_mut()
                .pop_front()
                .unwrap_or(Ok(Some(vec![])))
        };
        f(&Services {
            env: &self.env,
            probe: &probe,
            connect: &connect,
        })
    }

    fn connected(&self) -> usize {
        self.calls.borrow().len()
    }
}

fn ids(list: &[&str]) -> Reply {
    Ok(Some(list.iter().map(|s| s.to_string()).collect()))
}

fn rejected() -> Reply {
    Err(Error::new("provider.auth", "HTTP 401: bad key")
        .with_hint("check the API key reference in the provider settings"))
}

const SECRET: &str = "sk-the-secret-value-0123456789";

#[test]
fn the_list_puts_what_was_found_first_and_says_local_or_remote() {
    let m = Machine::new(MapEnv::new().with("OPENROUTER_API_KEY", SECRET), &[11434])
        .reply(ids(&["a", "b"]));
    // the first entry is the default; the model is typed; the remote consent is given
    let mut p = Scripted::new(&["", "y", "~openai/gpt-luna-latest", ""]);
    let chosen = m.run(|svc| guided(&mut p, svc)).unwrap();
    let out = p.output();
    let list: Vec<&str> = out.lines().filter(|l| l.contains(") ")).collect();
    assert!(
        list[0].contains("openrouter")
            && list[0].contains("remote")
            && list[0].contains("OPENROUTER_API_KEY is set"),
        "{out}"
    );
    assert!(
        list[1].contains("ollama")
            && list[1].contains("local")
            && list[1].contains("answering on localhost:11434"),
        "{out}"
    );
    assert!(list
        .iter()
        .any(|l| l.contains("openai") && l.contains("OPENAI_API_KEY is not set")));
    assert!(list.iter().any(|l| l.contains("custom")));
    // the key is in the environment, so it is not asked for, and its value is never shown
    assert!(
        !out.contains(SECRET) && !p.asked.iter().any(|q| q.contains("API key")),
        "{out}"
    );
    assert_eq!(chosen.main.preset.as_deref(), Some("openrouter"));
    assert_eq!(
        chosen.main.api_key.as_deref(),
        Some("env:OPENROUTER_API_KEY")
    );
    assert_eq!(chosen.main.model, "~openai/gpt-luna-latest");
    assert_eq!(chosen.main.trust, "remote");
    assert!(
        out.contains("connected; the server lists 2 models"),
        "{out}"
    );
}

#[test]
fn nothing_found_means_no_default_and_the_model_is_never_guessed() {
    let m = Machine::new(MapEnv::new(), &[]);
    // no default: an empty answer is refused
    let mut p = Scripted::new(&["", "", ""]);
    let e = m.run(|svc| guided(&mut p, svc)).unwrap_err();
    assert_eq!(e.code, "setup.no_answer");
    assert_eq!(m.connected(), 0);
    // a model has to be typed
    let m = Machine::new(MapEnv::new(), &[11434]);
    let mut p = Scripted::new(&["", "", "", "", ""]);
    let e = m.run(|svc| guided(&mut p, svc)).unwrap_err();
    assert_eq!(e.code, "setup.no_answer", "{}", p.output());
}

#[test]
fn a_local_server_confirms_its_port_and_another_one_becomes_an_endpoint() {
    let m = Machine::new(MapEnv::new(), &[11434]).reply(Ok(None));
    // pick ollama (default), keep the port, no key, model; then no checker
    let mut p = Scripted::new(&["", "", "", "my-local-model", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert_eq!(
        (c.main.preset.as_deref(), c.main.endpoint.as_deref()),
        (Some("ollama"), None)
    );
    assert_eq!((c.main.trust, c.main.api_key.as_deref()), ("local", None));
    assert!(p.output().contains("does not list its models"));

    let m = Machine::new(MapEnv::new(), &[11434]);
    let mut p = Scripted::new(&["", "11500", "", "m", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert_eq!(c.main.preset, None);
    assert_eq!(c.main.endpoint.as_deref(), Some("http://localhost:11500"));
    assert_eq!(
        m.calls.borrow()[0].1.endpoint.as_deref(),
        Some("http://localhost:11500")
    );
}

#[test]
fn a_remote_service_without_its_variable_asks_where_the_key_is() {
    let m = Machine::new(MapEnv::new(), &[]).reply(ids(&[]));
    // openrouter is item 1 of the table; consent; key reference typed; model
    let mut p = Scripted::new(&["1", "y", "file:~/.keys/openrouter", "some/model", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert!(p
        .output()
        .contains("OPENROUTER_API_KEY is not set in this shell"));
    assert_eq!(c.main.api_key.as_deref(), Some("file:~/.keys/openrouter"));
    // an empty answer takes the conventional variable
    let m = Machine::new(MapEnv::new(), &[]);
    let mut p = Scripted::new(&["1", "y", "", "some/model", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert_eq!(c.main.api_key.as_deref(), Some("env:OPENROUTER_API_KEY"));
}

#[test]
fn a_key_typed_by_mistake_is_refused_and_never_repeated() {
    let m = Machine::new(MapEnv::new(), &[]);
    // custom (item 6, after the five presets), a local address, the key itself (refused), then a reference
    let mut p = Scripted::new(&["6", "http://localhost:8000", SECRET, "env:MY_KEY", "m", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert_eq!(c.main.api_key.as_deref(), Some("env:MY_KEY"));
    assert!(p.output().contains("not a reference"), "{}", p.output());
    assert!(!p.output().contains(SECRET) && !format!("{:?}", c.main).contains(SECRET));
}

#[test]
fn declining_the_remote_consent_contacts_nothing() {
    let m = Machine::new(MapEnv::new().with("OPENROUTER_API_KEY", SECRET), &[]);
    let mut p = Scripted::new(&["", "n"]);
    let e = m.run(|svc| guided(&mut p, svc)).unwrap_err();
    assert_eq!(e.code, "setup.cancelled");
    assert_eq!(
        m.connected(),
        0,
        "the key must not be sent before the person agrees"
    );
}

#[test]
fn a_failed_connection_offers_to_try_again_continue_or_cancel() {
    let env = || MapEnv::new().with("OPENROUTER_API_KEY", SECRET);
    // try again with another key, which works
    let m = Machine::new(env(), &[])
        .reply(rejected())
        .reply(ids(&["x"]));
    let mut p = Scripted::new(&["", "y", "1", "env:OTHER_KEY", "x", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert!(p
        .output()
        .contains("could not confirm the connection: HTTP 401"));
    assert_eq!(c.main.api_key.as_deref(), Some("env:OTHER_KEY"));
    assert_eq!(m.connected(), 2);
    // continue anyway keeps what was entered
    let m = Machine::new(env(), &[]).reply(rejected());
    let mut p = Scripted::new(&["", "y", "2", "x", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert_eq!(c.main.api_key.as_deref(), Some("env:OPENROUTER_API_KEY"));
    // cancel
    let m = Machine::new(env(), &[]).reply(rejected());
    let mut p = Scripted::new(&["", "y", "3"]);
    assert_eq!(
        m.run(|svc| guided(&mut p, svc)).unwrap_err().code,
        "setup.cancelled"
    );
}

#[test]
fn a_model_missing_from_the_list_is_noted_and_kept() {
    let m =
        Machine::new(MapEnv::new().with("OPENROUTER_API_KEY", SECRET), &[]).reply(ids(&["a", "b"]));
    let mut p = Scripted::new(&["", "y", "~alias/latest", ""]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert_eq!(c.main.model, "~alias/latest");
    assert!(
        p.output().contains("not in the server's list"),
        "{}",
        p.output()
    );
}

#[test]
fn only_found_checkers_are_listed_and_none_is_the_default() {
    // typesafe has no key, openrouter has one, ollama answers: two checkers, and openai is never offered
    let env = MapEnv::new()
        .with("OPENROUTER_API_KEY", SECRET)
        .with("OPENAI_API_KEY", SECRET);
    let m = Machine::new(env, &[11434]).reply(Ok(None)).reply(Ok(None));
    let mut p = Scripted::new(&["", "y", "main-model", "", "", "", ""]);
    // main: openrouter; then the checker list, an empty answer is "none"
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    assert!(c.checker.is_none());
    let out = p.output();
    let after = out.split("A checker is").nth(1).unwrap();
    assert!(
        after.contains("openrouter") && after.contains("ollama") && after.contains("none"),
        "{out}"
    );
    assert!(
        !after.contains("typesafe") && !after.contains("openai "),
        "{out}"
    );
}

#[test]
fn a_remote_checker_is_set_up_like_a_provider_with_its_own_consent() {
    let env = MapEnv::new()
        .with("OPENROUTER_API_KEY", SECRET)
        .with("TYPESAFE_API_KEY", SECRET);
    let m = Machine::new(env, &[])
        .reply(ids(&["main"]))
        .reply(ids(&["jev-latest"]));
    // main: openrouter. The checker list follows the table (openrouter, typesafe, none): pick typesafe
    let mut p = Scripted::new(&["", "y", "main-model", "2", "y", "jev-latest"]);
    let c = m.run(|svc| guided(&mut p, svc)).unwrap();
    let checker = c.checker.unwrap();
    assert_eq!(checker.preset.as_deref(), Some("typesafe"));
    assert_eq!(checker.api_key.as_deref(), Some("env:TYPESAFE_API_KEY"));
    assert_eq!(checker.model, "jev-latest");
    assert!(p
        .output()
        .contains("your request, the command and the name of your shell"));
    assert_eq!(m.calls.borrow()[1].0, Kind::Checker);
}

#[test]
fn with_no_decision_model_found_the_checker_step_is_skipped_without_questions() {
    let m = Machine::new(MapEnv::new().with("OPENROUTER_API_KEY", SECRET), &[]);
    // openrouter is also a checker host, so use a machine where only a plain LLM key exists
    let m2 = Machine::new(MapEnv::new().with("OPENAI_API_KEY", SECRET), &[]);
    let mut p = Scripted::new(&["", "y", "model"]);
    let c = m2.run(|svc| guided(&mut p, svc)).unwrap();
    assert!(c.checker.is_none());
    assert!(p.output().contains("No decision model was found"));
    assert_eq!(p.left(), 0);
    let _ = m;
}

// ---- --yes ----------------------------------------------------------------------------------------------

fn args() -> SetupArgs {
    SetupArgs {
        yes: true,
        provider: None,
        model: None,
        shell: Some("bash".into()),
        base_url: None,
        api_key: None,
        api_key_env: None,
        checker: None,
        checker_model: None,
        checker_url: None,
        checker_api_key: None,
        verify: false,
    }
}

fn yes(a: &SetupArgs, m: &Machine) -> Result<Chosen> {
    m.run(|svc| from_args(a, svc))
}

#[test]
fn yes_needs_the_provider_and_the_model_and_guesses_neither() {
    let m = Machine::new(MapEnv::new(), &[11434]); // even a running Ollama is not a choice
    assert_eq!(yes(&args(), &m).unwrap_err().code, "setup.no_provider");
    let mut a = args();
    a.provider = Some("ollama".into());
    assert_eq!(yes(&a, &m).unwrap_err().code, "setup.no_model");
    a.model = Some("m".into());
    let c = yes(&a, &m).unwrap();
    assert_eq!(
        (c.main.preset.as_deref(), c.main.trust),
        (Some("ollama"), "local")
    );
    assert_eq!(m.connected(), 0, "--yes contacts nothing without --verify");
}

#[test]
fn yes_flags_cover_the_url_and_the_key_of_both_backends() {
    let m = Machine::new(MapEnv::new(), &[]);
    let mut a = args();
    a.provider = Some("custom".into());
    a.model = Some("m".into());
    assert_eq!(yes(&a, &m).unwrap_err().code, "setup.bad_base_url");
    a.base_url = Some("https://llm.example.com".into());
    a.api_key = Some("file:/run/keys/llm".into());
    a.checker = Some("typesafe".into());
    a.checker_model = Some("jev-latest".into());
    let c = yes(&a, &m).unwrap();
    assert_eq!(c.main.endpoint.as_deref(), Some("https://llm.example.com"));
    assert_eq!(
        (c.main.trust, c.main.api_key.as_deref()),
        ("remote", Some("file:/run/keys/llm"))
    );
    let ck = c.checker.unwrap();
    assert_eq!(
        (ck.preset.as_deref(), ck.api_key.as_deref()),
        (Some("typesafe"), Some("env:TYPESAFE_API_KEY"))
    );
    // a checker at an address of your own
    a.checker = Some("custom".into());
    a.checker_url = Some("http://localhost:9000".into());
    a.checker_api_key = Some("env:CHECK_KEY".into());
    let ck = yes(&a, &m).unwrap().checker.unwrap();
    assert_eq!(
        (ck.endpoint.as_deref(), ck.trust, ck.api_key.as_deref()),
        (
            Some("http://localhost:9000"),
            "local",
            Some("env:CHECK_KEY")
        )
    );
}

#[test]
fn a_url_replaces_the_presets_address_and_its_key_variable() {
    let m = Machine::new(MapEnv::new(), &[]);
    let mut a = args();
    a.provider = Some("openrouter".into());
    a.model = Some("m".into());
    a.base_url = Some("https://proxy.example".into());
    let c = yes(&a, &m).unwrap();
    assert_eq!(
        (c.main.preset, c.main.api_key),
        (None, None),
        "a key is never sent to an address it was not given for"
    );
}

#[test]
fn yes_refuses_a_key_that_is_not_a_reference_and_options_without_a_checker() {
    let m = Machine::new(MapEnv::new(), &[]);
    let mut a = args();
    a.provider = Some("openrouter".into());
    a.model = Some("m".into());
    a.api_key = Some(SECRET.into());
    let e = yes(&a, &m).unwrap_err();
    assert_eq!(e.code, "setup.bad_key_ref");
    assert!(!e.message.contains(SECRET) && !e.hint.unwrap_or_default().contains(SECRET));
    a.api_key = None;
    a.api_key_env = Some("MY_KEY".into());
    assert_eq!(
        yes(&a, &m).unwrap().main.api_key.as_deref(),
        Some("env:MY_KEY")
    );
    a.api_key = Some("env:OTHER".into());
    assert_eq!(
        yes(&a, &m).unwrap_err().code,
        "setup.bad_key_ref",
        "both forms at once"
    );
    a.api_key = None;
    a.checker_model = Some("x".into());
    assert_eq!(yes(&a, &m).unwrap_err().code, "setup.no_checker");
    a.checker_model = None;
    a.checker = Some("openai".into());
    a.checker_model = Some("x".into());
    let e = yes(&a, &m).unwrap_err();
    assert_eq!(
        e.code, "setup.unknown_provider",
        "openai does not serve the format"
    );
}

#[test]
fn verify_asks_each_server_and_a_failure_stops_setup() {
    let m = Machine::new(MapEnv::new(), &[])
        .reply(ids(&["m"]))
        .reply(rejected());
    let mut a = args();
    a.provider = Some("ollama".into());
    a.model = Some("m".into());
    a.checker = Some("typesafe".into());
    a.checker_model = Some("jev-latest".into());
    a.verify = true;
    let e = yes(&a, &m).unwrap_err();
    assert_eq!(e.code, "provider.auth");
    assert!(
        e.message.contains("typesafe did not answer"),
        "{}",
        e.message
    );
    assert_eq!(m.connected(), 2);
}

// ---- what is written ---------------------------------------------------------------------------------

#[test]
fn the_written_config_parses_and_carries_the_checker() {
    let main = Backend {
        preset: Some("openrouter".into()),
        endpoint: None,
        api_key: Some("env:OPENROUTER_API_KEY".into()),
        model: "~openai/gpt-luna-latest".into(),
        trust: "remote",
    };
    let checker = Backend {
        preset: Some("typesafe".into()),
        endpoint: None,
        api_key: Some("env:TYPESAFE_API_KEY".into()),
        model: "jev-latest".into(),
        trust: "remote",
    };
    let text = template_with(&main, Some(&checker), Some("bash")).unwrap();
    let c = Config::parse(&text).unwrap();
    assert_eq!(c.validator.chain, ["checker"]);
    assert!(c.models.contains_key("checker") && c.models.contains_key("main"));
    // a different service gets a provider of its own
    assert!(c.providers.contains_key("openrouter") && c.providers.contains_key("typesafe"));
    assert_eq!(c.models["checker"].provider, "typesafe");
    assert!(text.contains("preset = \"typesafe\"") && !text.contains(SECRET));
    // without a checker the validator keeps its commented example and stays off
    let plain = template_with(&main, None, Some("bash")).unwrap();
    let c = Config::parse(&plain).unwrap();
    assert!(c.validator.chain.is_empty() && !c.models.contains_key("checker"));
}

#[test]
fn a_checker_reached_the_same_way_shares_the_provider() {
    let main = Backend {
        preset: Some("openrouter".into()),
        endpoint: None,
        api_key: Some("env:OPENROUTER_API_KEY".into()),
        model: "~openai/gpt-luna-latest".into(),
        trust: "remote",
    };
    let checker = Backend {
        model: "~typesafe/jev-latest".into(),
        ..main.clone()
    };
    let text = template_with(&main, Some(&checker), Some("bash")).unwrap();
    let c = Config::parse(&text).unwrap();
    assert_eq!(c.providers.len(), 1, "{text}");
    assert_eq!(c.models["main"].provider, c.models["checker"].provider);
    assert_eq!(c.validator.chain, ["checker"]);
    // the written config builds, and a decision model is told apart from a chat model by its plugin
    c.validate_with(
        &shellmate_builtin::registry(),
        &shellmate_core::ports::Host::system(),
    )
    .ok();
}

#[test]
fn a_checker_on_another_connection_with_the_same_name_gets_its_own_provider() {
    let main = Backend {
        preset: Some("ollama".into()),
        endpoint: None,
        api_key: None,
        model: "a".into(),
        trust: "local",
    };
    let checker = Backend {
        trust: "remote",
        model: "b".into(),
        ..main.clone()
    };
    let c = Config::parse(&template_with(&main, Some(&checker), None).unwrap()).unwrap();
    assert_eq!(c.providers.len(), 2);
    assert_ne!(c.models["main"].provider, c.models["checker"].provider);
}
