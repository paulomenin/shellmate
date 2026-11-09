//! The command validator end to end: the real binary, the real `systemone` decider plugin, mock servers.

mod common;

use common::*;
use shellmate_testing::MockHttpServer;

fn systemone(match_p: f32, risk: &str) -> String {
    serde_json::json!({"model":"m","answers":{
        "match":{"type":"noul","noul":match_p},
        "risk":{"type":"choice","choice":risk,"confidence":0.9,
            "probabilities":{"read_only":0.0,"changes":0.0,"destructive":0.0}}},
        "usage":{"input_tokens":1,"output_tokens":0}})
    .to_string()
}

fn cfg(llm_url: &str, decider_url: &str, extra: &str) -> String {
    format!(
        "{}\n[validator]\nsemantic = true\ndeciders = [\"laya\"]\nllm_fallback = false\n\
         [deciders.laya]\nplugin = \"builtin:systemone\"\ntrust = \"local\"\n\
         [deciders.laya.settings]\nendpoint = \"{decider_url}\"\nmodel = \"laya\"\n{extra}",
        config(llm_url)
    )
}

const REPLY: &str = "Here.\n```shellmate-command\ndf -h\n```";

#[tokio::test]
async fn a_decision_model_finding_is_shown_and_the_command_still_lands_in_the_buffer() {
    let llm = MockHttpServer::start(200, &completion(REPLY)).await;
    let decider = MockHttpServer::start(200, &systemone(0.03, "read_only")).await;
    let c = cfg(&llm.url(), &decider.url(), "");
    let r = tokio::task::spawn_blocking(move || {
        run(&["invoke"], &c, &envelope("ask", "show disk usage"))
    })
    .await
    .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "df -h");
    assert!(
        r.stderr.contains("check: this may not be what you asked"),
        "{}",
        r.stderr
    );
    let sent = decider.last_request().await;
    assert!(
        sent.contains("/v1/systemone")
            && sent.contains("show disk usage")
            && sent.contains("df -h"),
        "{sent}"
    );
    assert_eq!(llm.request_count().await, 1, "no LLM call for the check");
}

#[tokio::test]
async fn an_unreachable_decider_is_reported_and_never_blocks_the_command() {
    let llm = MockHttpServer::start(200, &completion(REPLY)).await;
    let c = cfg(&llm.url(), "http://127.0.0.1:9", "");
    let r = tokio::task::spawn_blocking(move || {
        run(&["invoke"], &c, &envelope("ask", "show disk usage"))
    })
    .await
    .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "df -h");
    assert!(
        r.stderr.contains("the semantic check was skipped"),
        "{}",
        r.stderr
    );
}

#[tokio::test]
async fn a_remote_decider_only_sees_redacted_text() {
    let secret = "sk-abcdefghijklmnopqrstuvwxyz0123456789";
    let llm = MockHttpServer::start(200, &completion(REPLY)).await;
    let decider = MockHttpServer::start(200, &systemone(0.9, "read_only")).await;
    let c = remote_config(
        &llm.url(),
        &format!(
            "[validator]\nsemantic = true\ndeciders = [\"hosted\"]\nllm_fallback = false\n\
             [deciders.hosted]\nplugin = \"builtin:systemone\"\ntrust = \"remote\"\n\
             [deciders.hosted.settings]\nendpoint = \"{}\"\nmodel = \"m\"\n",
            decider.url()
        ),
    );
    let c2 = c.clone();
    let r = tokio::task::spawn_blocking(move || {
        run(
            &["invoke"],
            &c2,
            &envelope("ask", &format!("what is {secret} for disk usage")),
        )
    })
    .await
    .unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    let sent = decider.last_request().await;
    assert!(sent.contains("df -h"), "{sent}");
    assert!(
        !sent.contains(secret),
        "secret leaked to the decider: {sent}"
    );
    assert!(sent.contains("<SECRET:"), "{sent}");
}

#[tokio::test]
async fn a_remote_decider_is_refused_when_redaction_is_off() {
    let llm = MockHttpServer::start(200, &completion(REPLY)).await;
    let decider = MockHttpServer::start(200, &systemone(0.9, "read_only")).await;
    let c = format!(
        "{}\n[pipeline]\nllm = []\n[validator]\nsemantic = true\ndeciders = [\"hosted\"]\n\
         [deciders.hosted]\nplugin = \"builtin:systemone\"\ntrust = \"remote\"\n\
         [deciders.hosted.settings]\nendpoint = \"{}\"\nmodel = \"m\"\n",
        config(&llm.url()),
        decider.url()
    );
    let r = tokio::task::spawn_blocking(move || {
        run(&["invoke"], &c, &envelope("ask", "show disk usage"))
    })
    .await
    .unwrap();
    assert_ne!(r.code, 0);
    assert!(
        r.stderr.contains("privacy.no_redaction") && r.stderr.contains("hosted"),
        "{}",
        r.stderr
    );
    assert_eq!(decider.request_count().await, 0);
}

#[test]
fn config_check_rejects_unknown_deciders_and_bad_thresholds() {
    for (extra, want) in [
        ("[validator]\ndeciders = [\"ghost\"]\n", "unknown decider `ghost`"),
        ("[validator]\nmin_confidence = 0.2\n", "min_confidence"),
        ("[validator]\ntimeout = \"soon\"\n", "validator.timeout"),
        ("[deciders.x]\nplugin = \"builtin:nope\"\n[deciders.x.settings]\nmodel = \"m\"\nendpoint = \"http://127.0.0.1:1\"\n", "no decider plugin `nope`"),
        ("[deciders.x]\nplugin = \"builtin:systemone\"\n[deciders.x.settings]\nmodel = \"m\"\nendpoint = \"http://10.0.0.1:1\"\n", "non-loopback"),
    ] {
        let c = format!("{}\n{extra}", config("http://127.0.0.1:1"));
        let r = run(&["config", "check"], &c, b"");
        assert_ne!(r.code, 0, "{extra}");
        assert!(r.stderr.contains(want), "want {want:?} in {}", r.stderr);
    }
}
