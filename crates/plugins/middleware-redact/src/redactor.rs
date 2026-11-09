//! Replacing secrets in outgoing text, and putting them back in the answer.

use crate::{mapping::*, rules::*, *};
use regex::{Captures, Regex};
use sha2::{Digest, Sha256};
use shellmate_api::{Error, Result};

pub struct Redactor {
    pub(crate) rules: Vec<Rule>,
    pub(crate) key: [u8; 16],
}

impl Redactor {
    pub fn new(extra: &[ExtraPattern]) -> Result<Self> {
        let mut rules = builtin_rules();
        for p in extra {
            let re = Regex::new(&p.regex).map_err(|e| {
                Error::new(
                    "config.invalid",
                    format!("redact extra_patterns `{}`: {e}", p.name),
                )
            })?;
            rules.push(Rule {
                name: p.name.clone(),
                re,
                groups: &["secret"],
            });
        }
        Ok(Self {
            rules,
            key: random_key()?,
        })
    }

    /// Same redactor, fixed key (deterministic tests).
    pub fn with_key(mut self, key: [u8; 16]) -> Self {
        self.key = key;
        self
    }

    pub(crate) fn placeholder(&self, value: &str, map: &mut Mapping) -> String {
        if let Some(p) = map.forward.get(value) {
            return p.clone();
        }
        let mut h = Sha256::new();
        h.update(map.key.unwrap_or(self.key));
        h.update(value.as_bytes());
        let digest = h.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        // 6 hex chars; on a collision with a different value, take more of the digest
        for len in (6..=hex.len()).step_by(2) {
            let candidate = format!("{PREFIX}{}>", &hex[..len]);
            match map.reverse.get(&candidate) {
                None => {
                    map.forward.insert(value.to_string(), candidate.clone());
                    map.reverse.insert(candidate.clone(), value.to_string());
                    return candidate;
                }
                Some(existing) if existing == value => return candidate,
                Some(_) => continue,
            }
        }
        unreachable!("a full SHA-256 digest cannot collide")
    }

    /// Replace secrets in `text` with placeholders, recording them in `map`.
    pub fn redact(&self, text: &str, map: &mut Mapping) -> String {
        let mut current = text.to_string();
        for rule in &self.rules {
            current = self.apply_rule(rule, &current, map);
        }
        current
    }

    /// Apply one rule outside existing placeholders: a placeholder (`<SECRET:ab12cd>`) must never be
    /// matched by a later rule (its text looks like `secret: value`).
    pub(crate) fn apply_rule(&self, rule: &Rule, text: &str, map: &mut Mapping) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for ph in placeholder_re().find_iter(text) {
            out.push_str(&self.apply_rule_plain(rule, &text[last..ph.start()], map));
            out.push_str(ph.as_str());
            last = ph.end();
        }
        out.push_str(&self.apply_rule_plain(rule, &text[last..], map));
        out
    }

    pub(crate) fn apply_rule_plain(&self, rule: &Rule, text: &str, map: &mut Mapping) -> String {
        rule.re
            .replace_all(text, |caps: &Captures| {
                let whole = caps.get(0).expect("group 0");
                let secret = rule
                    .groups
                    .iter()
                    .find_map(|g| caps.name(g))
                    .unwrap_or(whole);
                if is_safe_value(secret.as_str()) {
                    return whole.as_str().to_string();
                }
                let ph = self.placeholder(secret.as_str(), map);
                let (w, s) = (whole.as_str(), secret);
                let rel = s.start() - whole.start()..s.end() - whole.start();
                format!("{}{}{}", &w[..rel.start], ph, &w[rel.end..])
            })
            .into_owned()
    }

    /// Put the original values back.
    pub fn restore(&self, text: &str, map: &Mapping) -> String {
        map.restore(text)
    }

    pub fn rule_names(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.name.as_str()).collect()
    }
}

pub(crate) fn random_key() -> Result<[u8; 16]> {
    use std::io::Read;
    let mut key = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut key))
        .map_err(|e| {
            Error::new(
                "redact.no_entropy",
                format!("cannot read /dev/urandom: {e}"),
            )
            .with_hint("redaction fails closed: nothing is sent without it")
        })?;
    Ok(key)
}
