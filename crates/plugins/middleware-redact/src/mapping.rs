//! The two-way mapping between secret values and their placeholders, for one call.

use crate::rules::*;
use std::collections::HashMap;

/// Original values and their placeholders for one conversation (here: one call).
#[derive(Default)]
pub struct Mapping {
    pub(crate) forward: HashMap<String, String>,
    pub(crate) reverse: HashMap<String, String>,
    /// Overrides the redactor's key: a conversation's key keeps placeholders stable across turns.
    pub(crate) key: Option<[u8; 16]>,
}

impl Mapping {
    pub fn with_key(key: Option<[u8; 16]>) -> Self {
        Self {
            key,
            ..Self::default()
        }
    }

    /// Put the original values back.
    pub fn restore(&self, text: &str) -> String {
        if self.is_empty() || !text.contains(PREFIX) {
            return text.to_string();
        }
        let mut out = text.to_string();
        // longest placeholders first so a short one cannot clip a longer one
        let mut phs: Vec<&String> = self.reverse.keys().collect();
        phs.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for ph in phs {
            out = out.replace(ph.as_str(), &self.reverse[ph]);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.forward.len()
    }
    pub fn is_empty(&self) -> bool {
        self.forward.is_empty()
    }
}
