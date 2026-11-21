//! Running a tool that implements `shellmate_api::Tool`.
//!
//! This is where the core enforces. The tool says what a call needs and does the work; everything that
//! keeps a person in charge happens here, whatever the tool says:
//!
//! - a tool that declares it needs approval cannot skip it (INV-1);
//! - a pre-approval rule applies only to a call the tool marks preapprovable, on a turn that is not
//!   tainted, and the rules are the user's (INV-1, INV-4);
//! - only the approver, which only the binary implements, can say yes;
//! - a tool that declares untrusted output taints the turn, whatever it returns (INV-4);
//! - what a tool writes is escaped before a terminal sees it.

use super::services::CoreServices;
use super::{ToolDecision, ToolEnv, ToolRun};
use crate::present::{Mark, Notice, Tone};
use crate::sanitize;
use crate::tools::{ApprovalRequest, Choice, ToolCall};
use shellmate_api as api;
use std::sync::Arc;

/// A tool and the settings the core keeps for it.
#[derive(Clone)]
pub(in crate::engine) struct Plugged {
    pub(in crate::engine) tool: Arc<dyn api::Tool>,
    pub(in crate::engine) settings: api::ToolSettings,
}

impl Plugged {
    pub(in crate::engine) fn name(&self) -> &'static str {
        self.tool.info().name
    }

    /// The tool is on and may be offered and used.
    fn available(&self) -> bool {
        self.settings.enabled && self.settings.approval != api::ApprovalSetting::Deny
    }

    pub(in crate::engine) fn offer(&self, action: api::ToolAction) -> Option<String> {
        self.available()
            .then(|| self.tool.offer(action, &self.settings))
            .flatten()
    }

    /// Rules of the user's that are too broad, as `(rule, why)`, for `config check`.
    pub(in crate::engine) fn risky_rules(
        &self,
        host: &crate::ports::Host,
    ) -> Vec<(String, String)> {
        self.tool
            .risky_rules(&self.settings.preapproved, &CoreServices::new(host))
    }

    /// Decide and, if allowed, run one call. A refusal is a `ToolRun`, not an error.
    pub(in crate::engine) async fn run(&self, call: &ToolCall, env: &ToolEnv<'_>) -> ToolRun {
        let info = self.tool.info();
        if !self.available() {
            return refused(api::Refusal::denied(
                info.name,
                &call.rest,
                &format!("the {} tool is disabled.", info.name),
            ));
        }
        let services = CoreServices::new(env.host);
        let ctx = api::ToolCtx {
            settings: &self.settings,
            services: &services,
            cwd: env.cwd,
            tainted: env.tainted,
            dest: env.dest,
        };
        let api_call = api::ToolCall {
            name: call.name.clone(),
            args: call.args.clone(),
            rest: call.rest.clone(),
        };
        let (prepared, granted, decision) = match self.tool.assess(&api_call, &ctx).await {
            api::Assessment::Refuse(r) => return refused(r),
            api::Assessment::Allow(p) => {
                if info.approval == api::ApprovalMode::Required {
                    // The tool said it needs a person, then tried to go without one.
                    return refused(api::Refusal::denied(
                        info.name,
                        &call.rest,
                        "the tool needs approval and did not ask for it.",
                    ));
                }
                (p, api::Granted::NotRequired, ToolDecision::Allow)
            }
            api::Assessment::NeedsApproval(need) => match self.approve(*need, &services, env) {
                Ok(go) => go,
                Err(run) => return *run,
            },
        };
        let out = self.tool.run(prepared, &granted, &ctx).await;
        let mut run = ToolRun::new(notice(out.notice), out.result, decision);
        run.untrusted = out.untrusted || info.untrusted_output;
        if out.failed {
            run.decision = ToolDecision::Deny;
        }
        run
    }

    /// Pre-approval, then the person. `Err` is a call that does not go ahead.
    fn approve(
        &self,
        need: api::ApprovalNeed,
        services: &CoreServices<'_>,
        env: &ToolEnv<'_>,
    ) -> Result<(api::Prepared, api::Granted, ToolDecision), Box<ToolRun>> {
        let rule = (need.preapprovable && !env.tainted)
            .then(|| {
                self.settings
                    .preapproved
                    .iter()
                    .find(|r| self.tool.rule_matches(r, &need.rule_subject, services))
            })
            .flatten();
        if let Some(rule) = rule {
            return Ok((
                need.prepared,
                api::Granted::Rule(rule.clone()),
                ToolDecision::Preapproved,
            ));
        }
        let request = ApprovalRequest {
            kind: need.kind,
            command: need.subject,
            cwd: need.cwd,
            risk: need.risk,
            reasons: need.reasons,
            can_buffer: env.buffer_ok && need.to_buffer.is_some(),
        };
        match env.approver.map(|a| a.approve(&request)) {
            Some(Choice::Run) => Ok((need.prepared, api::Granted::Person, ToolDecision::Approved)),
            Some(Choice::ToBuffer) => {
                Err(Box::new(match need.to_buffer.filter(|_| env.buffer_ok) {
                    Some(tb) => {
                        let mut run = refused_as(tb.refusal, ToolDecision::ToBuffer);
                        run.to_buffer = Some(tb.buffer);
                        run
                    }
                    None => refused_as(need.denied, ToolDecision::DeniedByUser),
                }))
            }
            Some(Choice::Deny) => Err(Box::new(refused_as(
                need.denied,
                ToolDecision::DeniedByUser,
            ))),
            None => Err(Box::new(refused_as(
                need.no_terminal,
                ToolDecision::DeniedNoTerminal,
            ))),
        }
    }
}

fn refused(r: api::Refusal) -> ToolRun {
    refused_as(r, ToolDecision::Deny)
}

fn refused_as(r: api::Refusal, decision: ToolDecision) -> ToolRun {
    ToolRun::new(notice(r.notice), r.result, decision)
}

/// A tool's notice as the presenter takes it. The text is the tool's, so it is escaped here.
fn notice(n: api::ToolNotice) -> Notice {
    let tone = match n.tone {
        api::Tone::Info => Tone::Info,
        api::Tone::Success => Tone::Success,
        api::Tone::Warning => Tone::Warning,
        api::Tone::Error => Tone::Error,
        api::Tone::Muted => Tone::Muted,
    };
    let mark = match n.mark {
        api::Mark::Check => Mark::Check,
        api::Mark::Cross => Mark::Cross,
        api::Mark::Arrow => Mark::Arrow,
        api::Mark::Chevron => Mark::Chevron,
    };
    let mut out = Notice::new(tone, mark, sanitize::escape(&n.text)).with_detail(
        n.detail
            .iter()
            .map(|l| sanitize::escape(l).to_string())
            .collect(),
    );
    if let Some(meta) = n.meta {
        out = out.with_meta(sanitize::escape(&meta));
    }
    out
}
