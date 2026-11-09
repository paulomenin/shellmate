//! `conversation list/show/new/...`.

use super::finish_text;
use crate::convo;
use shellmate_core::Config;

pub(crate) fn run(
    config: Option<&std::path::Path>,
    session: Option<&str>,
    command: convo::ConvoCmd,
) -> i32 {
    finish_text(
        Config::load(config)
            .and_then(|cfg| convo::run(command, &cfg, session, super::ui::stdout().as_ref())),
    )
}
