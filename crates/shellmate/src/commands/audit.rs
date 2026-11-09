//! `audit show|export|clear`.

use super::{finish_text, ui};
use crate::admin;
use crate::cli::AuditCmd;
use shellmate_core::Config;

pub(crate) fn run(config: Option<&std::path::Path>, command: AuditCmd) -> i32 {
    let (action, last, session) = match command {
        AuditCmd::Show { last, session } => ("show", last, session),
        AuditCmd::Export { last, session } => ("export", last, session),
        AuditCmd::Clear => ("clear", None, None),
    };
    finish_text(Config::load(config).and_then(|cfg| {
        admin::audit_cmd(
            &cfg,
            action,
            last,
            session.as_deref(),
            ui::stdout().as_ref(),
        )
    }))
}
