//! The actions the engine runs, as the shell glue names them.

use crate::sanitize;
use shellmate_api::{Error, Result};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Ask,
    Generate,
    Explain,
    /// Forget the session's conversation; the next request starts a new one. Never calls a model.
    NewConversation,
}

impl Action {
    /// The name the glue sends and the config uses (`[actions.<name>]`).
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Ask => "ask",
            Action::Generate => "generate",
            Action::Explain => "explain",
            Action::NewConversation => "new-conversation",
        }
    }
}

impl FromStr for Action {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "ask" => Ok(Action::Ask),
            "generate" => Ok(Action::Generate),
            "explain" => Ok(Action::Explain),
            "new-conversation" => Ok(Action::NewConversation),
            other => Err(Error::new(
                "glue.protocol",
                format!("unknown action `{}`", sanitize::escape(other)),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_round_trips_through_its_name() {
        for a in [
            Action::Ask,
            Action::Generate,
            Action::Explain,
            Action::NewConversation,
        ] {
            assert_eq!(a.as_str().parse::<Action>().unwrap(), a);
        }
    }

    #[test]
    fn an_unknown_name_is_a_protocol_error_that_cannot_carry_escapes() {
        let e = "fly\x1b[31m".parse::<Action>().unwrap_err();
        assert_eq!(e.code, "glue.protocol");
        assert!(!e.message.contains('\x1b'));
    }
}
