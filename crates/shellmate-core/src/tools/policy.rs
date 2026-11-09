//! The policy for `man`: the classifier first (it can only make things stricter), then the setting.

use super::call::ToolCall;
use crate::config::{Approval, ManTool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Safe,
    /// Must not run without a human decision. Carries the reasons.
    Unsafe(Vec<String>),
}

pub(super) fn is_section(s: &str) -> bool {
    matches!(s.as_bytes(), [d, rest @ ..] if d.is_ascii_digit() && rest.len() <= 3 && rest.iter().all(u8::is_ascii_lowercase))
        || s == "n"
        || s == "l"
}

pub(super) fn is_page(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"_.+:@-".contains(c))
}

/// The minimal classifier. It can only make things stricter: `Safe` approves nothing by itself.
/// For `man` it rejects everything that is not a plain page (and optional section): options such as
/// `-P`/`--pager` run programs, and anything with a `/` makes `man` open an arbitrary file.
pub fn classify(call: &ToolCall) -> Verdict {
    match call.name.as_str() {
        "man" => {
            let ok = match call.args.as_slice() {
                [page] => is_page(page),
                [section, page] => is_section(section) && is_page(page),
                _ => false,
            };
            if ok {
                Verdict::Safe
            } else {
                Verdict::Unsafe(vec![
                    "only `man [section] page` with a plain page name is allowed".into(),
                ])
            }
        }
        other => Verdict::Unsafe(vec![format!("unknown tool `{other}`")]),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
}

#[derive(Debug, Clone)]
pub struct ToolPolicy {
    pub man: ManTool,
}

impl ToolPolicy {
    pub fn new(man: ManTool) -> Self {
        Self { man }
    }

    pub fn enabled(&self) -> bool {
        self.man.enabled && self.man.approval == Approval::Allow
    }

    /// Classifier first (an `Unsafe` verdict always needs a human, and there is no prompt yet, so it is
    /// a denial), then the tool's own approval setting.
    pub fn decide(&self, call: &ToolCall) -> Decision {
        if let Verdict::Unsafe(reasons) = classify(call) {
            return Decision::Deny(format!(
                "not allowed without approval: {}",
                reasons.join("; ")
            ));
        }
        match (call.name.as_str(), self.man.enabled, self.man.approval) {
            ("man", true, Approval::Allow) => Decision::Allow,
            ("man", _, _) => Decision::Deny("the man tool is disabled".into()),
            _ => Decision::Deny("unknown tool".into()),
        }
    }
}
