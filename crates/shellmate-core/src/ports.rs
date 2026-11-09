//! The outside world as the core sees it: environment variables, the clock and the file system.
//!
//! The core asks these ports instead of the operating system, so a test can say what `PATH` is, what time
//! it is and which files exist without touching the machine. Production uses [`Host::system`].
//!
//! The environment that `exec` gives its child is built from [`Env`] (an allow list of variables); the
//! "inherit" setting passes the real process environment on purpose.
//!
//! What is not behind a port, on purpose:
//! - Starting programs (`exec`, `man`, `git`) and the monotonic clock used for timings. Those are the
//!   behaviour, not a lookup, and a fake would only test the fake.
//! - The `read` tool's path checks (`tools::prepare_read`). Symlink and `..` resolution against the real
//!   file system is exactly what its tests must see, and an in-memory file system does not model it.
//! - Writes: the audit log, the capture file and the conversation store are adapters of their own.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Environment variables. Values that are not valid UTF-8 read as unset.
pub trait Env: Send + Sync {
    fn var(&self, name: &str) -> Option<String>;

    /// A variable that is set and not empty (the usual meaning of `XDG_*` and `HOME`).
    fn nonempty(&self, name: &str) -> Option<String> {
        self.var(name).filter(|v| !v.is_empty())
    }
}

/// The process environment.
pub struct SystemEnv;

impl Env for SystemEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var_os(name).and_then(|v| v.into_string().ok())
    }
}

/// A fixed set of variables.
#[derive(Debug, Clone, Default)]
pub struct MapEnv(BTreeMap<String, String>);

impl MapEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.0.insert(name.to_string(), value.to_string());
        self
    }
}

impl Env for MapEnv {
    fn var(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

/// Wall-clock time.
pub trait Clock: Send + Sync {
    fn unix_nanos(&self) -> u128;

    fn unix_secs(&self) -> u64 {
        (self.unix_nanos() / 1_000_000_000) as u64
    }
}

/// The system clock. A clock set before 1970 reads as 0.
pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_nanos(&self) -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn at(secs: u64) -> Self {
        Self(AtomicU64::new(secs))
    }

    pub fn advance(&self, secs: u64) {
        self.0.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn unix_nanos(&self) -> u128 {
        self.0.load(Ordering::SeqCst) as u128 * 1_000_000_000
    }
}

/// What the core wants to know about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub is_file: bool,
    pub is_dir: bool,
    pub len: u64,
    /// Unix permission bits.
    pub mode: u32,
    pub uid: u32,
}

/// Reading from the file system.
pub trait Fs: Send + Sync {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// Like `std::fs::metadata`: follows a final symlink.
    fn metadata(&self, path: &Path) -> io::Result<Meta>;
    /// Like `std::fs::symlink_metadata`: describes the link itself.
    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta>;

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        String::from_utf8(self.read(path)?).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "stream did not contain valid UTF-8",
            )
        })
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.metadata(path).is_ok_and(|m| m.is_dir)
    }
}

/// The real file system.
pub struct RealFs;

fn meta_of(m: &std::fs::Metadata) -> Meta {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    Meta {
        is_file: m.is_file(),
        is_dir: m.is_dir(),
        len: m.len(),
        mode: m.permissions().mode(),
        uid: m.uid(),
    }
}

impl Fs for RealFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn metadata(&self, path: &Path) -> io::Result<Meta> {
        std::fs::metadata(path).map(|m| meta_of(&m))
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        std::fs::symlink_metadata(path).map(|m| meta_of(&m))
    }
}

/// A file in memory: content, permission bits, owner.
type MemFile = (Vec<u8>, u32, u32);

/// Files and directories in memory. There are no symlinks: `metadata` and `symlink_metadata` agree.
#[derive(Default)]
pub struct MemFs {
    files: Mutex<BTreeMap<PathBuf, MemFile>>,
    dirs: Mutex<BTreeSet<PathBuf>>,
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// A file owned by `uid` with permission bits `mode`.
    pub fn file(self, path: &str, content: &str, mode: u32, uid: u32) -> Self {
        self.files.lock().expect("memfs lock").insert(
            PathBuf::from(path),
            (content.as_bytes().to_vec(), mode, uid),
        );
        self
    }

    pub fn dir(self, path: &str) -> Self {
        self.dirs
            .lock()
            .expect("memfs lock")
            .insert(PathBuf::from(path));
        self
    }

    fn find(&self, path: &Path) -> io::Result<Meta> {
        if let Some((bytes, mode, uid)) = self.files.lock().expect("memfs lock").get(path) {
            return Ok(Meta {
                is_file: true,
                is_dir: false,
                len: bytes.len() as u64,
                mode: *mode,
                uid: *uid,
            });
        }
        if self.dirs.lock().expect("memfs lock").contains(path) {
            return Ok(Meta {
                is_file: false,
                is_dir: true,
                len: 0,
                mode: 0o755,
                uid: 0,
            });
        }
        Err(io::ErrorKind::NotFound.into())
    }
}

impl Fs for MemFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.files.lock().expect("memfs lock").get(path) {
            Some((bytes, _, _)) => Ok(bytes.clone()),
            None => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn metadata(&self, path: &Path) -> io::Result<Meta> {
        self.find(path)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        self.find(path)
    }
}

/// The three ports together, passed around as one.
#[derive(Clone)]
pub struct Host {
    pub env: Arc<dyn Env>,
    pub clock: Arc<dyn Clock>,
    pub fs: Arc<dyn Fs>,
}

impl Host {
    /// The real machine.
    pub fn system() -> Self {
        Self {
            env: Arc::new(SystemEnv),
            clock: Arc::new(SystemClock),
            fs: Arc::new(RealFs),
        }
    }

    pub fn with_env(mut self, env: impl Env + 'static) -> Self {
        self.env = Arc::new(env);
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_fs(mut self, fs: impl Fs + 'static) -> Self {
        self.fs = Arc::new(fs);
        self
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::system()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_env_reads_what_it_was_given_and_treats_empty_as_unset_on_request() {
        let e = MapEnv::new().with("A", "x").with("B", "");
        assert_eq!(e.var("A").as_deref(), Some("x"));
        assert_eq!(e.var("B").as_deref(), Some(""));
        assert_eq!(e.nonempty("B"), None);
        assert_eq!(e.var("C"), None);
    }

    #[test]
    fn the_manual_clock_moves_only_when_told() {
        let c = ManualClock::at(100);
        assert_eq!(c.unix_secs(), 100);
        c.advance(5);
        assert_eq!(c.unix_secs(), 105);
        assert_eq!(c.unix_nanos(), 105_000_000_000);
    }

    #[test]
    fn the_system_clock_is_after_2020() {
        assert!(SystemClock.unix_secs() > 1_577_836_800);
    }

    #[test]
    fn mem_fs_reports_files_dirs_and_missing_paths() {
        let fs = MemFs::new()
            .file("/a/key", "secret\n", 0o600, 1000)
            .dir("/a");
        let m = fs.metadata(Path::new("/a/key")).unwrap();
        assert!(m.is_file && !m.is_dir);
        assert_eq!((m.len, m.mode, m.uid), (7, 0o600, 1000));
        assert_eq!(fs.read_to_string(Path::new("/a/key")).unwrap(), "secret\n");
        assert!(fs.is_dir(Path::new("/a")));
        assert!(!fs.is_dir(Path::new("/a/key")));
        assert_eq!(
            fs.read(Path::new("/nope")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn real_fs_agrees_with_std() {
        let dir = std::env::temp_dir();
        assert!(RealFs.is_dir(&dir));
        assert!(RealFs.metadata(&dir.join("sm-no-such-file-xyz")).is_err());
    }
}
