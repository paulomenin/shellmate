//! The audit log (`observability.md` §3): append-only JSON lines of what shellmate did on the user's
//! behalf. Content-free: no prompts, answers or tool results; only decisions, sizes, timings and codes.
//! Volatile (`$XDG_RUNTIME_DIR`) unless `[audit] persist = true`.

use crate::config::AuditConfig;
use crate::ports::{Clock, Env, Host, SystemClock};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const FORMAT_VERSION: u32 = 1;

/// Where the engine sends audit events. The file log is [`Audit`]; [`MemoryAudit`] keeps them in memory
/// (tests, embedding). Recording never fails the request: a sink swallows its own errors.
pub trait AuditSink: Send + Sync {
    fn record(&self, event: &str, ctx: &Value, fields: Value);
}

/// The JSON object for one event: format version, time, name, then the request context and the fields.
fn event_object(ts: u64, event: &str, ctx: &Value, fields: Value) -> Value {
    let mut obj = json!({"v": FORMAT_VERSION, "ts": ts, "event": event});
    for src in [ctx, &fields] {
        if let (Some(o), Some(s)) = (obj.as_object_mut(), src.as_object()) {
            for (k, v) in s {
                o.insert(k.clone(), v.clone());
            }
        }
    }
    obj
}

/// An in-memory sink: the events, in order.
pub struct MemoryAudit {
    events: std::sync::Mutex<Vec<Value>>,
    clock: std::sync::Arc<dyn Clock>,
}

impl Default for MemoryAudit {
    fn default() -> Self {
        Self::with_clock(std::sync::Arc::new(SystemClock))
    }
}

impl MemoryAudit {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stamp events with `clock` instead of the system time.
    pub fn with_clock(clock: std::sync::Arc<dyn Clock>) -> Self {
        Self {
            events: Default::default(),
            clock,
        }
    }

    pub fn events(&self) -> Vec<Value> {
        self.events.lock().expect("audit lock").clone()
    }
}

impl AuditSink for MemoryAudit {
    fn record(&self, event: &str, ctx: &Value, fields: Value) {
        let obj = event_object(self.clock.unix_secs(), event, ctx, fields);
        self.events.lock().expect("audit lock").push(obj);
    }
}

impl<T: AuditSink + ?Sized> AuditSink for std::sync::Arc<T> {
    fn record(&self, event: &str, ctx: &Value, fields: Value) {
        (**self).record(event, ctx, fields)
    }
}

/// The audit log in a file.
pub struct Audit {
    path: PathBuf,
    retention_secs: u64,
    max_bytes: u64,
    persist: bool,
    clock: std::sync::Arc<dyn Clock>,
}

fn env_dir(env: &dyn Env, var: &str) -> Option<PathBuf> {
    env.nonempty(var).map(PathBuf::from)
}

/// Where the log lives for this configuration.
pub fn log_path(cfg: &AuditConfig, env: &dyn Env) -> PathBuf {
    if cfg.persist {
        let state = env_dir(env, "XDG_STATE_HOME")
            .or_else(|| env_dir(env, "HOME").map(|h| h.join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        state.join("shellmate/audit/audit.jsonl")
    } else {
        let run = env_dir(env, "XDG_RUNTIME_DIR").unwrap_or_else(|| {
            // SAFETY: getuid has no preconditions.
            PathBuf::from(format!("/tmp/shellmate-{}", unsafe { libc::getuid() }))
        });
        run.join("shellmate/audit.jsonl")
    }
}

impl Audit {
    /// `None` when the log is turned off.
    pub fn from_config(cfg: &AuditConfig, host: &Host) -> Option<Self> {
        cfg.enabled.then(|| Self {
            path: log_path(cfg, host.env.as_ref()),
            clock: host.clock.clone(),
            retention_secs: crate::config::parse_duration(&cfg.retention)
                .map(|d| d.as_secs())
                .unwrap_or(30 * 86400),
            max_bytes: cfg.max_bytes,
            persist: cfg.persist,
        })
    }

    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            retention_secs: 30 * 86400,
            max_bytes: 10 * 1024 * 1024,
            persist: false,
            clock: std::sync::Arc::new(SystemClock),
        }
    }

    /// Stamp events and judge their age with `clock` instead of the system time.
    pub fn with_clock(mut self, clock: std::sync::Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Record one event. Failures never affect the request (an audit problem is not a user problem).
    pub fn record(&self, event: &str, ctx: &Value, fields: Value) {
        let obj = event_object(self.clock.unix_secs(), event, ctx, fields);
        let Ok(line) = serde_json::to_string(&obj) else {
            return;
        };
        if let Some(dir) = self.path.parent() {
            if std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .is_err()
            {
                return;
            }
        }
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.path)
        {
            let _ = f.write_all(format!("{line}\n").as_bytes());
        }
        if self.persist {
            self.prune();
        }
    }

    /// Drop entries older than the retention, then the oldest ones until the file fits the size cap.
    fn prune(&self) {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return;
        };
        if meta.len() <= self.max_bytes {
            // cheap path: only look at the age when the first line could be expired
            let Ok(text) = std::fs::read_to_string(&self.path) else {
                return;
            };
            let first_ts = text
                .lines()
                .next()
                .and_then(|l| serde_json::from_str::<Value>(l).ok())
                .and_then(|v| v["ts"].as_u64())
                .unwrap_or(u64::MAX);
            if self.clock.unix_secs().saturating_sub(first_ts) <= self.retention_secs {
                return;
            }
        }
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return;
        };
        let cutoff = self.clock.unix_secs().saturating_sub(self.retention_secs);
        let mut kept: Vec<&str> = text
            .lines()
            .filter(|l| {
                serde_json::from_str::<Value>(l)
                    .ok()
                    .and_then(|v| v["ts"].as_u64())
                    .is_some_and(|t| t >= cutoff)
            })
            .collect();
        let mut size: u64 = kept.iter().map(|l| l.len() as u64 + 1).sum();
        while size > self.max_bytes && !kept.is_empty() {
            size -= kept.remove(0).len() as u64 + 1;
        }
        let tmp = self.path.with_extension("tmp");
        let body: String = kept.iter().map(|l| format!("{l}\n")).collect();
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
        {
            if f.write_all(body.as_bytes()).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}

impl AuditSink for Audit {
    fn record(&self, event: &str, ctx: &Value, fields: Value) {
        Audit::record(self, event, ctx, fields)
    }
}

/// Read events (oldest first), optionally for one session and only the last `last` of them.
pub fn read(path: &Path, session: Option<&str>, last: Option<usize>) -> Vec<Value> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| session.is_none_or(|s| v["session"].as_str() == Some(s)))
        .collect();
    if let Some(n) = last {
        let skip = out.len().saturating_sub(n);
        out.drain(..skip);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sm-audit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("audit.jsonl")
    }

    #[test]
    fn events_are_appended_with_context_and_private() {
        let p = tmp("a");
        let a = Audit::at(p.clone());
        a.record(
            "request_started",
            &json!({"session": "s1"}),
            json!({"action": "ask"}),
        );
        a.record(
            "turn_finished",
            &json!({"session": "s2"}),
            json!({"outcome": "completed"}),
        );
        let all = read(&p, None, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["event"], "request_started");
        assert_eq!(all[0]["session"], "s1");
        assert_eq!(all[0]["v"], FORMAT_VERSION);
        assert_eq!(read(&p, Some("s2"), None).len(), 1);
        assert_eq!(read(&p, None, Some(1))[0]["event"], "turn_finished");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o077,
            0
        );
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn persistent_logs_drop_expired_and_oversized_entries() {
        let p = tmp("b");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let old = SystemClock.unix_secs() - 40 * 86400;
        std::fs::write(&p, format!("{{\"v\":1,\"ts\":{old},\"event\":\"old\"}}\n")).unwrap();
        let mut a = Audit::at(p.clone());
        a.persist = true;
        a.record("new", &json!({}), json!({}));
        let all = read(&p, None, None);
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!(all[0]["event"], "new");

        a.max_bytes = 200;
        for _ in 0..20 {
            a.record("filler", &json!({}), json!({"n": "xxxxxxxxxx"}));
        }
        assert!(std::fs::metadata(&p).unwrap().len() <= 200 + 100);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn unwritable_locations_are_silent() {
        let a = Audit::at(PathBuf::from("/proc/nope/audit.jsonl"));
        a.record("x", &json!({}), json!({})); // must not panic
    }
}
