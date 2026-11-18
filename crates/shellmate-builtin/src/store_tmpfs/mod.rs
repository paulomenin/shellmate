//! `builtin:store-tmpfs`: one JSONL file per conversation (header line, then one turn per line),
//! a pointer file per session, advisory `flock` locks. Everything is `0600` in a `0700` directory
//! that lives on tmpfs (`$XDG_RUNTIME_DIR`), so transcripts disappear at logout.

use serde::Deserialize;
use serde_json::Value;
use shellmate_api::{
    busy, ConversationInfo, ConversationLock, Current, Error, Registry, Result, Store,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const PLUGIN_ID: &str = "store-tmpfs";

/// The store's settings. Unknown keys and a wrong type are errors (with `config.invalid`, not
/// `conversation.store`, which the engine takes to mean "conversations are off"), so a typo cannot move the
/// transcripts somewhere else.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    /// Where the store lives. Default: `$XDG_RUNTIME_DIR/shellmate/store`.
    dir: Option<PathBuf>,
}

pub fn register(registry: &mut Registry) {
    registry.register_store(
        PLUGIN_ID,
        Box::new(|settings| {
            let s: Settings = serde_json::from_value(settings.clone())
                .map_err(|e| Error::new("config.invalid", e.to_string()))?;
            let dir = s.dir.unwrap_or_else(default_dir);
            Ok(Arc::new(TmpfsStore::open(dir)?) as Arc<dyn Store>)
        }),
    );
}

fn default_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("shellmate").join("store"),
        // SAFETY: getuid has no preconditions.
        _ => PathBuf::from(format!("/tmp/shellmate-{}", unsafe { libc::getuid() })).join("store"),
    }
}

pub struct TmpfsStore {
    conv: PathBuf,
    sess: PathBuf,
}

fn io_err(what: &str, e: std::io::Error) -> Error {
    Error::new("conversation.store", format!("{what}: {e}"))
}

/// Create `dir` (and parents) as `0700` and refuse a directory another user could read or own.
fn private_dir(dir: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| io_err(&format!("creating {}", dir.display()), e))?;
    let meta = fs::metadata(dir).map_err(|e| io_err(&format!("checking {}", dir.display()), e))?;
    // SAFETY: getuid has no preconditions.
    let me = unsafe { libc::getuid() };
    if meta.uid() != me || meta.permissions().mode() & 0o077 != 0 {
        return Err(Error::new(
            "conversation.store",
            format!(
                "{} is not private (owner or permissions); refusing to store conversations there",
                dir.display()
            ),
        )
        .with_hint("remove it or point `dir` at a directory only you can access"));
    }
    Ok(())
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn checked(kind: &str, s: &str) -> Result<()> {
    if valid_name(s) {
        Ok(())
    } else {
        Err(Error::new(
            "conversation.bad_id",
            format!("invalid {kind} id"),
        ))
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn mtime(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_id() -> Result<String> {
    let mut b = [0u8; 8];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| io_err("reading /dev/urandom", e))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

impl TmpfsStore {
    pub fn open(dir: PathBuf) -> Result<Self> {
        let conv = dir.join("conv");
        let sess = dir.join("sess");
        // parents (including the shared `shellmate` dir) must be private too
        if let Some(parent) = dir.parent() {
            if parent.file_name().is_some_and(|n| n == "shellmate") {
                private_dir(parent)?;
            }
        }
        private_dir(&dir)?;
        private_dir(&conv)?;
        private_dir(&sess)?;
        Ok(Self { conv, sess })
    }

    fn file(&self, id: &str) -> Result<PathBuf> {
        checked("conversation", id)?;
        Ok(self.conv.join(format!("{id}.jsonl")))
    }

    fn lines(&self, id: &str) -> Result<Vec<String>> {
        let text = fs::read_to_string(self.file(id)?).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::new("conversation.not_found", format!("no conversation `{id}`"))
            } else {
                io_err("reading conversation", e)
            }
        })?;
        Ok(text.lines().map(str::to_string).collect())
    }

    /// Replace the file's content atomically (write a temp file, then rename).
    fn rewrite(&self, id: &str, lines: &[String]) -> Result<()> {
        let path = self.file(id)?;
        let tmp = self.conv.join(format!("{id}.tmp"));
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| io_err("writing conversation", e))?;
        for l in lines {
            writeln!(f, "{l}").map_err(|e| io_err("writing conversation", e))?;
        }
        f.sync_all().ok();
        fs::rename(&tmp, &path).map_err(|e| io_err("replacing conversation", e))
    }
}

struct FlockGuard(File);
impl ConversationLock for FlockGuard {}
impl Drop for FlockGuard {
    fn drop(&mut self) {
        // SAFETY: the fd is open for the lifetime of `self`; closing it would also release the lock.
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

impl Store for TmpfsStore {
    fn current(&self, session: &str) -> Result<Option<Current>> {
        checked("session", session)?;
        let Ok(id) = fs::read_to_string(self.sess.join(session)) else {
            return Ok(None);
        };
        let id = id.trim();
        let Ok(meta) = self
            .file(id)
            .and_then(|p| fs::metadata(p).map_err(|e| io_err("checking conversation", e)))
        else {
            return Ok(None); // pointer to a deleted conversation
        };
        Ok(Some(Current {
            conversation: id.to_string(),
            last_activity: mtime(&meta),
        }))
    }

    fn set_current(&self, session: &str, conversation: &str) -> Result<()> {
        checked("session", session)?;
        checked("conversation", conversation)?;
        let tmp = self.sess.join(format!("{session}.tmp"));
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| io_err("writing session pointer", e))?;
        f.write_all(conversation.as_bytes())
            .map_err(|e| io_err("writing session pointer", e))?;
        fs::rename(&tmp, self.sess.join(session)).map_err(|e| io_err("writing session pointer", e))
    }

    fn create(&self, header: &Value) -> Result<String> {
        let id = random_id()?;
        let mut header = header.clone();
        if let Some(o) = header.as_object_mut() {
            o.insert("id".into(), Value::String(id.clone()));
            o.insert("created".into(), Value::from(now()));
        }
        let line = serde_json::to_string(&header)
            .map_err(|e| Error::new("conversation.store", e.to_string()))?;
        self.rewrite(&id, &[line])?;
        Ok(id)
    }

    fn read(&self, conversation: &str, tail: Option<usize>) -> Result<(Value, Vec<Value>)> {
        let mut lines = self.lines(conversation)?.into_iter();
        let header = lines
            .next()
            .and_then(|l| serde_json::from_str(&l).ok())
            .ok_or_else(|| Error::new("conversation.corrupt", "conversation has no header"))?;
        let mut turns: Vec<Value> = lines
            .filter_map(|l| serde_json::from_str(&l).ok()) // a torn line is skipped, not fatal
            .collect();
        if let Some(n) = tail {
            let skip = turns.len().saturating_sub(n);
            turns.drain(..skip);
        }
        Ok((header, turns))
    }

    fn append_turn(&self, conversation: &str, turn: &Value, max_turns: usize) -> Result<()> {
        let line = serde_json::to_string(turn)
            .map_err(|e| Error::new("conversation.store", e.to_string()))?;
        let path = self.file(conversation)?;
        let mut f = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| io_err("opening conversation", e))?;
        // one write call: the line is either fully there or not at all for concurrent readers
        f.write_all(format!("{line}\n").as_bytes())
            .map_err(|e| io_err("appending turn", e))?;
        drop(f);
        let lines = self.lines(conversation)?;
        if lines.len() > max_turns + 1 {
            let mut kept = vec![lines[0].clone()];
            kept.extend_from_slice(&lines[lines.len() - max_turns..]);
            self.rewrite(conversation, &kept)?;
        }
        Ok(())
    }

    fn pop_turn(&self, conversation: &str) -> Result<Option<Value>> {
        let mut lines = self.lines(conversation)?;
        if lines.len() <= 1 {
            return Ok(None);
        }
        let last = lines.pop().expect("non-empty");
        self.rewrite(conversation, &lines)?;
        Ok(serde_json::from_str(&last).ok())
    }

    fn list(&self) -> Result<Vec<ConversationInfo>> {
        let mut out = Vec::new();
        let rd = fs::read_dir(&self.conv).map_err(|e| io_err("listing conversations", e))?;
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".jsonl") else {
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            let turns = self
                .lines(id)
                .map(|l| l.len().saturating_sub(1))
                .unwrap_or(0);
            out.push(ConversationInfo {
                id: id.to_string(),
                turns,
                last_activity: mtime(&meta),
            });
        }
        out.sort_by(|a, b| b.last_activity.cmp(&a.last_activity).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    fn delete(&self, conversation: &str) -> Result<()> {
        let path = self.file(conversation)?;
        fs::remove_file(&path).map_err(|e| io_err("deleting conversation", e))?;
        let _ = fs::remove_file(self.conv.join(format!("{conversation}.lock")));
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        for dir in [&self.conv, &self.sess] {
            for entry in fs::read_dir(dir)
                .map_err(|e| io_err("clearing", e))?
                .flatten()
            {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }

    fn lock(&self, conversation: &str, timeout: Duration) -> Result<Box<dyn ConversationLock>> {
        checked("conversation", conversation)?;
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(self.conv.join(format!("{conversation}.lock")))
            .map_err(|e| io_err("opening lock", e))?;
        let start = Instant::now();
        loop {
            // SAFETY: the fd is valid for the duration of the call.
            if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Box::new(FlockGuard(f)));
            }
            if start.elapsed() >= timeout {
                return Err(busy());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn store() -> (TmpfsStore, PathBuf) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sm-store-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        (TmpfsStore::open(dir.join("store")).unwrap(), dir)
    }

    fn built(settings: Value) -> Result<Arc<dyn Store>> {
        let mut r = Registry::new();
        register(&mut r);
        r.build_store(PLUGIN_ID, &settings)
    }

    #[test]
    fn the_settings_are_strict_and_a_bad_one_creates_nothing() {
        let (_, dir) = store();
        let target = dir.join("elsewhere");
        for bad in [
            json!({"dir": target.to_str().unwrap(), "unknown_setting": true}),
            json!({"dirr": target.to_str().unwrap()}),
            json!({"dir": 5}),
            json!({"dir": ["a"]}),
        ] {
            let err = built(bad.clone()).err().expect("refused");
            // not `conversation.store`: the engine takes that to mean "conversations are off"
            assert_eq!(err.code, "config.invalid", "{bad}: {}", err.message);
        }
        assert!(
            !target.exists(),
            "a refused setting must not create a directory"
        );
        assert!(built(json!({"dir": target.to_str().unwrap()})).is_ok());
        assert!(target.is_dir());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_tmpfs_store_meets_the_store_contract() {
        let (s, dir) = store();
        shellmate_testing::assert_store_contract(&s);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_append_read_in_order() {
        let (s, dir) = store();
        let id = s.create(&json!({"session": "s1"})).unwrap();
        for i in 0..5 {
            s.append_turn(&id, &json!({"n": i}), 40).unwrap();
        }
        let (h, turns) = s.read(&id, None).unwrap();
        assert_eq!(h["session"], "s1");
        assert_eq!(h["id"], id.as_str());
        assert_eq!(turns.len(), 5);
        assert_eq!(turns[4]["n"], 4);
        let (_, tail) = s.read(&id, Some(2)).unwrap();
        assert_eq!(
            tail.iter()
                .map(|t| t["n"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [3, 4]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn max_turns_drops_the_oldest_and_keeps_the_header() {
        let (s, dir) = store();
        let id = s.create(&json!({})).unwrap();
        for i in 0..7 {
            s.append_turn(&id, &json!({"n": i}), 3).unwrap();
        }
        let (h, turns) = s.read(&id, None).unwrap();
        assert_eq!(h["id"], id.as_str());
        assert_eq!(
            turns
                .iter()
                .map(|t| t["n"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [4, 5, 6]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn session_pointer_pop_delete_and_clear() {
        let (s, dir) = store();
        assert_eq!(s.current("sess1").unwrap(), None);
        let id = s.create(&json!({})).unwrap();
        s.set_current("sess1", &id).unwrap();
        let cur = s.current("sess1").unwrap().unwrap();
        assert_eq!(cur.conversation, id);
        s.append_turn(&id, &json!({"n": 1}), 40).unwrap();
        s.append_turn(&id, &json!({"n": 2}), 40).unwrap();
        assert_eq!(s.pop_turn(&id).unwrap().unwrap()["n"], 2);
        assert_eq!(s.read(&id, None).unwrap().1.len(), 1);
        assert_eq!(s.list().unwrap()[0].turns, 1);
        s.delete(&id).unwrap();
        assert_eq!(
            s.current("sess1").unwrap(),
            None,
            "dangling pointer is ignored"
        );
        let id2 = s.create(&json!({})).unwrap();
        s.set_current("sess1", &id2).unwrap();
        s.clear().unwrap();
        assert!(s.list().unwrap().is_empty());
        assert_eq!(s.current("sess1").unwrap(), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ids_cannot_escape_the_directory() {
        let (s, dir) = store();
        assert_eq!(
            s.read("../../etc/passwd", None).unwrap_err().code,
            "conversation.bad_id"
        );
        assert_eq!(s.current("a/b").unwrap_err().code, "conversation.bad_id");
        assert_eq!(
            s.set_current("ok", "../x").unwrap_err().code,
            "conversation.bad_id"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lock_excludes_a_second_holder_until_released() {
        let (s, dir) = store();
        let id = s.create(&json!({})).unwrap();
        let first = s.lock(&id, Duration::from_millis(50)).unwrap();
        let err = s.lock(&id, Duration::from_millis(100)).err().unwrap();
        assert_eq!(err.code, "conversation.busy");
        drop(first);
        assert!(s.lock(&id, Duration::from_millis(100)).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn files_are_private() {
        let (s, dir) = store();
        let id = s.create(&json!({})).unwrap();
        let mode = fs::metadata(s.file(&id).unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_shared_directory_is_refused() {
        let dir = std::env::temp_dir().join(format!("sm-store-open-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let err = TmpfsStore::open(dir.clone()).err().unwrap();
        assert_eq!(err.code, "conversation.store");
        fs::remove_dir_all(dir).unwrap();
    }
}
