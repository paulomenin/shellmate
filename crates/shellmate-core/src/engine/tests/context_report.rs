//! `shellmate context`: which pieces of context each provider would get, and why the rest are held back.
//! Sizes and reasons only, never content.

use super::*;
use crate::capture::{Capture, MAX_AGE_SECS};
use crate::engine::{ContextItem, ContextReport, ItemState};
use crate::ports::{Host, ManualClock, MapEnv, MemFs};

fn full_env() -> Envelope {
    let mut e = env("ask", "list big files");
    e.shell_version = Some("4.1".into());
    e.status = Some(2);
    e.hist = vec![(0, "ls".into()), (1, "make".into())];
    e
}

async fn report_at(level: u8, cap: u8, mut cfg: Config, e: &Envelope) -> ContextReport {
    cfg.context.default_level = level;
    let engine = Engine::with_providers(
        vec![ProviderSpec {
            name: "a".into(),
            provider: FakeProvider::replying("x") as Arc<dyn Provider>,
            max_level: cap,
            trust: Trust::Local,
        }],
        level,
    )
    .with_os("TestOS")
    .with_config(cfg);
    engine.context_report(e).await.unwrap().remove(0)
}

fn item<'a>(r: &'a ContextReport, name: &str) -> &'a ContextItem {
    r.items
        .iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("no item `{name}` in {:?}", r.items))
}

fn is_sent(i: &ContextItem) -> bool {
    i.state == ItemState::Sent
}

#[tokio::test]
async fn the_items_come_in_a_fixed_order_with_the_level_that_unlocks_each() {
    let r = report_at(4, 4, Config::defaults(), &full_env()).await;
    let seen: Vec<_> = r.items.iter().map(|i| (i.name, i.level)).collect();
    assert_eq!(
        seen,
        [
            ("your typed text", 0),
            ("shell and system", 1),
            ("working directory", 1),
            ("project type", 1),
            ("last exit status", 2),
            ("recent commands", 2),
            ("git state", 2),
            ("captured output", 2),
        ]
    );
    assert_eq!((r.provider.as_str(), r.level, r.ceiling), ("a", 4, 4));
    assert!(!r.redaction && !r.tainted);
}

#[tokio::test]
async fn level_one_sends_the_basics_and_holds_back_the_rest_naming_the_setting() {
    let r = report_at(1, 4, Config::defaults(), &full_env()).await;
    let typed = item(&r, "your typed text");
    assert!(is_sent(typed) && typed.detail == "14 chars", "{typed:?}");
    let shell = item(&r, "shell and system");
    assert!(
        is_sent(shell) && shell.detail == "fish 4.1 on TestOS",
        "{shell:?}"
    );
    assert_eq!(item(&r, "working directory").detail, "/work/x");
    for name in [
        "last exit status",
        "recent commands",
        "git state",
        "captured output",
    ] {
        let i = item(&r, name);
        assert!(!is_sent(i), "{name}");
        assert_eq!(i.detail, "needs level 2; [context] level is 1", "{name}");
    }
}

#[tokio::test]
async fn a_provider_ceiling_is_named_when_it_is_what_holds_context_back() {
    let r = report_at(3, 1, Config::defaults(), &full_env()).await;
    assert_eq!((r.level, r.ceiling), (1, 1));
    let status = item(&r, "last exit status");
    assert!(!is_sent(status));
    assert_eq!(
        status.detail,
        "needs level 2; local providers are capped at 1"
    );
}

#[tokio::test]
async fn level_two_sends_what_the_shell_reported() {
    let r = report_at(2, 4, Config::defaults(), &full_env()).await;
    assert_eq!(item(&r, "last exit status").detail, "status 2");
    assert_eq!(
        item(&r, "recent commands").detail,
        "2 command line(s), up to 5"
    );
    // nothing to send: the reason is the absence, not the level
    assert_eq!(item(&r, "git state").detail, "not a git repository");
    assert!(item(&r, "captured output")
        .detail
        .starts_with("nothing captured"));
    assert!(!r.tainted);
}

#[tokio::test]
async fn a_shell_that_reported_nothing_says_so() {
    let bare = env("ask", "x");
    let r = report_at(2, 4, Config::defaults(), &bare).await;
    assert_eq!(
        item(&r, "last exit status").detail,
        "the shell did not report one"
    );
    assert_eq!(item(&r, "recent commands").detail, "none reported");
    let mut no_cwd = full_env();
    no_cwd.cwd = None;
    let r = report_at(1, 4, Config::defaults(), &no_cwd).await;
    assert_eq!(
        item(&r, "working directory").detail,
        "the shell did not report one"
    );
}

#[tokio::test]
async fn switched_off_collectors_name_their_setting() {
    let mut cfg = Config::defaults();
    cfg.context.project = false;
    cfg.context.git = false;
    let r = report_at(2, 4, cfg, &full_env()).await;
    assert_eq!(
        item(&r, "project type").detail,
        "turned off ([context] project = false)"
    );
    assert_eq!(
        item(&r, "git state").detail,
        "turned off ([context] git = false)"
    );
}

fn captured(at: u64) -> (Host, Envelope) {
    // SAFETY: getuid has no preconditions.
    let me = unsafe { libc::getuid() };
    let cap = Capture {
        command: "make".into(),
        status: Some(2),
        at,
        truncated: false,
        output: "boom\n".into(),
    };
    let fs = MemFs::new().file(
        "/run/u/shellmate/capture/s",
        &serde_json::to_string(&cap).unwrap(),
        0o600,
        me,
    );
    let host = Host::system()
        .with_env(MapEnv::new().with("XDG_RUNTIME_DIR", "/run/u"))
        .with_clock(Arc::new(ManualClock::at(at + 10)))
        .with_fs(fs);
    let mut e = full_env();
    e.session = Some("s".into());
    (host, e)
}

async fn report_with_capture(level: u8, host: Host, e: &Envelope) -> ContextReport {
    let engine =
        engine_at_level(FakeProvider::replying("x"), level, Config::defaults()).with_host(host);
    engine.context_report(e).await.unwrap().remove(0)
}

#[tokio::test]
async fn captured_output_is_listed_by_size_and_taints_the_request_from_level_two() {
    let (host, e) = captured(1_000);
    let r = report_with_capture(2, host.clone(), &e).await;
    let cap = item(&r, "captured output");
    assert!(is_sent(cap), "{cap:?}");
    assert!(
        cap.detail.starts_with("last `shellmate run`, "),
        "{}",
        cap.detail
    );
    assert!(r.tainted, "captured output is text a program wrote");
    // below level 2 it is neither sent nor a reason to distrust the request
    let r = report_with_capture(1, host, &e).await;
    assert!(!is_sent(item(&r, "captured output")));
    assert!(!r.tainted);
}

#[tokio::test]
async fn an_old_capture_is_not_offered() {
    let (host, e) = captured(1_000);
    let later = host.with_clock(Arc::new(ManualClock::at(1_000 + MAX_AGE_SECS + 5)));
    let r = report_with_capture(2, later, &e).await;
    assert!(item(&r, "captured output")
        .detail
        .starts_with("nothing captured"));
    assert!(!r.tainted);
}

#[tokio::test]
async fn every_provider_in_the_chain_gets_its_own_report() {
    let mk = |name: &str, cap: u8| ProviderSpec {
        name: name.into(),
        provider: FakeProvider::replying("x") as Arc<dyn Provider>,
        max_level: cap,
        trust: Trust::Local,
    };
    let engine = Engine::with_providers(vec![mk("big", 4), mk("small", 1)], 3).with_os("TestOS");
    let reports = engine.context_report(&full_env()).await.unwrap();
    let levels: Vec<_> = reports
        .iter()
        .map(|r| (r.provider.as_str(), r.level))
        .collect();
    assert_eq!(levels, [("big", 3), ("small", 1)]);
}

#[tokio::test]
async fn nothing_to_report_without_a_provider_and_remote_needs_redaction() {
    let none = Engine::with_providers(Vec::new(), 1);
    assert_eq!(
        none.context_report(&env("ask", "x"))
            .await
            .unwrap_err()
            .code,
        "config.no_provider"
    );
    let remote = Engine::with_providers(remote(FakeProvider::replying("x")), 1);
    assert_eq!(
        remote
            .context_report(&env("ask", "x"))
            .await
            .unwrap_err()
            .code,
        "privacy.no_redaction"
    );
}
