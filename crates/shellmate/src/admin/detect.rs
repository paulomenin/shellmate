//! What `setup` can tell about this machine before it asks anything: which hosted services have a key in
//! the environment (by name only, never the value) and which local servers answer on their default port.
//!
//! The candidates come from the presets, so a new preset shows up here without a change.

use super::doctor::reachable;
use super::templates::is_loopback_url;
use shellmate_api::Preset;
use shellmate_core::ports::Env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// A server on this machine, at the preset's default port.
    Local { port: u16 },
    /// A hosted service, whose key is conventionally in `key_env`.
    Remote { key_env: Option<&'static str> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub preset: &'static str,
    pub place: Place,
    /// A local server answered on its port, or the hosted service's key variable is set.
    pub detected: bool,
}

impl Candidate {
    /// One line for the list: local or remote, and what was found. Never the value of a variable.
    pub fn describe(&self) -> String {
        match (&self.place, self.detected) {
            (Place::Local { port }, true) => format!("local   answering on localhost:{port}"),
            (Place::Local { port }, false) => format!("local   nothing on localhost:{port}"),
            (Place::Remote { key_env: Some(v) }, true) => format!("remote  {v} is set"),
            (Place::Remote { key_env: Some(v) }, false) => format!("remote  {v} is not set"),
            (Place::Remote { key_env: None }, _) => "remote".to_string(),
        }
    }
}

/// Where a preset's server is: on this machine, at its default port, or hosted.
pub fn place(p: &Preset) -> Place {
    if is_loopback_url(p.endpoint) {
        Place::Local {
            port: port_of(p.endpoint),
        }
    } else {
        Place::Remote { key_env: p.key_env }
    }
}

fn port_of(url: &str) -> u16 {
    let authority = url
        .split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or("");
    authority
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
        .unwrap_or(if url.starts_with("https") { 443 } else { 80 })
}

/// Each preset as a candidate, with what is found on this machine. The ones found come first, and the rest
/// keep the order of the table.
pub fn detect(
    presets: impl IntoIterator<Item = &'static Preset>,
    env: &dyn Env,
    probe: &dyn Fn(u16) -> bool,
) -> Vec<Candidate> {
    let mut found: Vec<Candidate> = presets
        .into_iter()
        .map(|p| {
            let place = place(p);
            let detected = match &place {
                Place::Local { port } => probe(*port),
                Place::Remote { key_env } => key_env.is_some_and(|v| env.nonempty(v).is_some()),
            };
            Candidate {
                preset: p.name,
                place,
                detected,
            }
        })
        .collect();
    found.sort_by_key(|c| !c.detected);
    found
}

/// Does anything accept a connection on this port of this machine?
pub fn probe_port(port: u16) -> bool {
    reachable(&format!("localhost:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellmate_core::ports::MapEnv;

    static TABLE: &[Preset] = &[
        Preset::new("hosted", "https://api.example.com", Some("HOSTED_KEY")),
        Preset::new("other", "https://other.example.com", Some("OTHER_KEY")),
        Preset::new("local-a", "http://localhost:1111", None),
        Preset::new("local-b", "http://localhost:2222", None),
    ];

    #[test]
    fn what_is_found_comes_first_and_the_rest_keep_the_table_order() {
        let env = MapEnv::new().with("OTHER_KEY", "sk-the-secret-value");
        let c = detect(TABLE, &env, &|port| port == 2222);
        let names: Vec<_> = c.iter().map(|c| (c.preset, c.detected)).collect();
        assert_eq!(
            names,
            [
                ("other", true),
                ("local-b", true),
                ("hosted", false),
                ("local-a", false)
            ]
        );
        assert!(
            matches!(c[1].place, Place::Local { .. }) && matches!(c[0].place, Place::Remote { .. })
        );
    }

    #[test]
    fn the_probe_gets_each_local_ports_and_only_those() {
        let seen = std::cell::RefCell::new(Vec::new());
        detect(TABLE, &MapEnv::new(), &|p| {
            seen.borrow_mut().push(p);
            false
        });
        assert_eq!(*seen.borrow(), [1111, 2222]);
    }

    #[test]
    fn a_description_says_local_or_remote_and_never_shows_a_value() {
        let env = MapEnv::new().with("HOSTED_KEY", "sk-the-secret-value");
        let text: Vec<String> = detect(TABLE, &env, &|p| p == 1111)
            .iter()
            .map(Candidate::describe)
            .collect();
        assert!(text.iter().any(|t| t == "remote  HOSTED_KEY is set"));
        assert!(text.iter().any(|t| t == "remote  OTHER_KEY is not set"));
        assert!(text
            .iter()
            .any(|t| t == "local   answering on localhost:1111"));
        assert!(text
            .iter()
            .any(|t| t == "local   nothing on localhost:2222"));
        assert!(text.iter().all(|t| !t.contains("sk-the-secret-value")));
    }

    #[test]
    fn an_empty_variable_is_not_a_key() {
        let env = MapEnv::new().with("HOSTED_KEY", "");
        let c = detect(TABLE, &env, &|_| false);
        assert!(c.iter().all(|c| !c.detected));
    }

    #[test]
    fn the_real_presets_are_all_either_local_or_have_a_key_variable() {
        for p in shellmate_api::PRESETS {
            let c = detect([p], &MapEnv::new(), &|_| false);
            match &c[0].place {
                Place::Local { port } => assert!(*port > 0, "{}", p.name),
                Place::Remote { key_env } => assert!(key_env.is_some(), "{}", p.name),
            }
        }
    }
}
