//! `shellmate context`: what each provider would be told about your shell, piece by piece. Sizes and reasons
//! only; `--full` adds the exact payload after redaction (the same text `invoke --dry-run` prints).

use shellmate_core::config::Trust;
use shellmate_core::engine::{ContextReport, ItemState};
use shellmate_core::present::{Presenter, Tone};

pub fn render(reports: &[ContextReport], utf8: bool, p: &dyn Presenter) -> String {
    let (yes, no) = if utf8 { ("✓", "·") } else { ("+", "-") };
    let mut o = String::new();
    for r in reports {
        o.push_str(&format!(
            "provider `{}` ({}) · context level {} (ceiling {}) · redaction {}\n",
            p.label(Tone::Info, &r.provider),
            match r.trust {
                Trust::Local => p.label(Tone::Success, "local"),
                Trust::Remote => p.label(Tone::Warning, "remote"),
            },
            r.level,
            r.ceiling,
            if r.redaction {
                p.label(Tone::Success, "on")
            } else {
                p.label(Tone::Muted, "off")
            }
        ));
        let width = r.items.iter().map(|i| i.name.len()).max().unwrap_or(0);
        for i in &r.items {
            let sent = i.state == ItemState::Sent;
            let mark = if sent {
                p.label(Tone::Success, yes)
            } else {
                p.label(Tone::Muted, no)
            };
            let name = format!("{:<width$}", i.name);
            let (name, detail) = if sent {
                (name, p.label(Tone::Info, &i.detail))
            } else {
                (p.label(Tone::Muted, &name), p.label(Tone::Muted, &i.detail))
            };
            o.push_str(&format!("  {mark} L{} {name}  {detail}\n", i.level));
        }
        if r.tainted {
            o.push_str(&format!(
                "  {}\n",
                p.label(
                    Tone::Warning,
                    "! this request carries text written by others: pre-approved exec and read are suspended"
                )
            ));
        }
        o.push('\n');
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellmate_core::engine::ContextItem;
    use shellmate_core::present::{Caps, PlainPresenter};

    #[test]
    fn marks_sent_and_withheld_and_flags_taint() {
        let r = ContextReport {
            provider: "p".into(),
            trust: Trust::Remote,
            level: 1,
            ceiling: 1,
            redaction: true,
            tainted: true,
            items: vec![
                ContextItem {
                    name: "working directory",
                    level: 1,
                    state: ItemState::Sent,
                    detail: "~/x".into(),
                },
                ContextItem {
                    name: "git state",
                    level: 2,
                    state: ItemState::Withheld,
                    detail: "needs level 2".into(),
                },
            ],
        };
        let t = render(&[r], false, &PlainPresenter::new(Caps::plain(80, false)));
        assert!(t.contains("+ L1 working directory"), "{t}");
        assert!(t.contains("- L2 git state"), "{t}");
        assert!(t.contains("pre-approved exec and read are suspended"));
        assert!(t.contains("remote"));
    }
}
