//! Engine tests. Shared helpers live here; the tests are grouped by concern in the child modules.

mod audit;
mod context;
mod context_report;
mod exec;
mod golden_tools;
mod notices;
mod read;
mod run;
mod secrets;
mod tool_config;
mod tool_decisions;
mod tool_host;
mod turns;
mod validator;

use super::*;
use crate::config::{Config, Trust};
use crate::envelope::Envelope;
use crate::ports::{Host, MapEnv};
use crate::tools::{ApprovalKind, ApprovalRequest, Approver, Choice};
use async_trait::async_trait;
use shellmate_api::{
    ChatRequest, ChatStream, Middleware, Next, Provider, ProviderError, ProviderErrorKind, Result,
};
use shellmate_testing::FakeProvider;
use std::sync::{Arc, Mutex};

impl Engine {
    /// The tools of the built-in registry, built from the engine's config.
    fn with_builtin_tools(self) -> Self {
        self.with_tools_from(&shellmate_builtin::registry())
            .unwrap()
    }
}

/// The `[tools.<name>]` entry of `cfg`, made if it is not there.
fn tool<'a>(cfg: &'a mut Config, name: &str) -> &'a mut crate::config::ToolEntry {
    cfg.tools.entries.entry(name.into()).or_default()
}

/// Turn the `man` tool off in `cfg`.
fn man_off(cfg: &mut Config) {
    tool(cfg, "man").enabled = false;
}

fn env(action: &str, buffer: &str) -> Envelope {
    Envelope {
        version: 1,
        action: action.into(),
        shell: "fish".into(),
        cwd: Some("/work/x".into()),
        buffer: buffer.into(),
        ..Default::default()
    }
}

fn engine(list: Vec<(&str, Arc<FakeProvider>, u8)>) -> Engine {
    Engine::with_providers(
        list.into_iter()
            .map(|(n, p, l)| ProviderSpec {
                name: n.to_string(),
                provider: p as Arc<dyn Provider>,
                max_level: l,
                trust: Trust::Local,
            })
            .collect(),
        1,
    )
    .with_os("TestOS")
    .with_builtin_tools()
}

fn remote(p: Arc<FakeProvider>) -> Vec<ProviderSpec> {
    vec![ProviderSpec {
        name: "cloud".into(),
        provider: p as Arc<dyn Provider>,
        max_level: 1,
        trust: Trust::Remote,
    }]
}

struct Passthrough;
#[async_trait]
impl Middleware<ChatRequest, ChatStream> for Passthrough {
    fn id(&self) -> &str {
        "test:passthrough"
    }
    async fn call(
        &self,
        req: ChatRequest,
        next: Next<'_, ChatRequest, ChatStream>,
    ) -> Result<ChatStream> {
        next.run(req).await
    }
}

/// A layer that answers by itself and never passes the request on (`next` zero times).
struct ShortCircuit;
#[async_trait]
impl Middleware<ChatRequest, ChatStream> for ShortCircuit {
    fn id(&self) -> &str {
        "test:short-circuit"
    }
    async fn call(
        &self,
        _req: ChatRequest,
        _next: Next<'_, ChatRequest, ChatStream>,
    ) -> Result<ChatStream> {
        Ok(shellmate_api::stream_from(Vec::new()))
    }
}

fn repo_dir(name: &str, subject: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sm-ctx-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("Cargo.toml"), "").unwrap();
    let g = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&d)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    g(&["init", "-q", "-b", "trunk"]);
    g(&["add", "Cargo.toml"]);
    g(&["commit", "-q", "-m", subject]);
    std::fs::write(d.join("scratch.txt"), "x").unwrap();
    d
}

fn engine_at_level(p: Arc<FakeProvider>, level: u8, mut cfg: Config) -> Engine {
    cfg.context.default_level = level;
    Engine::with_providers(
        vec![ProviderSpec {
            name: "a".into(),
            provider: p as Arc<dyn Provider>,
            max_level: 4,
            trust: Trust::Local,
        }],
        level,
    )
    .with_os("TestOS")
    .with_config(cfg)
    .with_builtin_tools()
}

fn system_prompt(p: &FakeProvider) -> String {
    p.seen.lock().unwrap()[0].messages[0].content.clone()
}

fn have_git() -> bool {
    crate::needs::have("git")
}

struct FakeApprover {
    choice: Choice,
    asked: Mutex<Vec<ApprovalRequest>>,
}
impl FakeApprover {
    fn new(choice: Choice) -> Arc<Self> {
        Arc::new(Self {
            choice,
            asked: Mutex::new(Vec::new()),
        })
    }
    fn count(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}
impl Approver for FakeApprover {
    fn approve(&self, request: &ApprovalRequest) -> Choice {
        self.asked.lock().unwrap().push(request.clone());
        self.choice
    }
}

fn exec_call(cmd: &str) -> String {
    format!("```shellmate-tool\nexec {cmd}\n```")
}

fn second_request(p: &FakeProvider) -> String {
    let seen = p.seen.lock().unwrap();
    seen[1].messages.last().unwrap().content.clone()
}
