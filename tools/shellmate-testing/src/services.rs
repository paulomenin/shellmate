//! A stand-in for the core's [`ToolServices`], so a tool can be tested without the core: no process is
//! started and no file is read unless the test says what comes back.

use async_trait::async_trait;
use shellmate_api::{
    CommandRisk, FileTarget, FileText, ProcessOutput, ProcessSpec, Risk, ToolServices,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

type Process = Box<dyn Fn(&ProcessSpec) -> ProcessOutput + Send + Sync>;
type Classify = Box<dyn Fn(&str) -> CommandRisk + Send + Sync>;
type Vet = Box<dyn Fn(&str) -> Result<FileTarget, String> + Send + Sync>;
type Read = Box<dyn Fn(&FileTarget, usize) -> Result<FileText, String> + Send + Sync>;

pub struct FakeServices {
    pub env: BTreeMap<String, String>,
    /// Directories that exist.
    pub dirs: Vec<String>,
    /// Programs found by name.
    pub programs: BTreeMap<String, PathBuf>,
    /// What `classify_command` answers. Default: safe, low risk.
    pub classify: Classify,
    /// What `vet_file` answers for an argument. Default: an error.
    pub vet: Vet,
    /// What `read_file` answers. Default: an error.
    pub read: Read,
    /// What a started program produces. Default: exit 0 with no output.
    pub process: Process,
    /// Every program the tool asked to start, in order.
    pub started: Mutex<Vec<ProcessSpec>>,
}

impl Default for FakeServices {
    fn default() -> Self {
        Self {
            env: BTreeMap::new(),
            dirs: Vec::new(),
            programs: BTreeMap::new(),
            classify: Box::new(|_| CommandRisk {
                safe: true,
                risk: Risk::Low,
                reasons: Vec::new(),
            }),
            vet: Box::new(|a| Err(format!("no such file `{a}`"))),
            read: Box::new(|t, _| Err(format!("cannot read `{}`", t.shown))),
            process: Box::new(|_| output(Some(0), "", "")),
            started: Mutex::new(Vec::new()),
        }
    }
}

/// A finished program's output.
pub fn output(exit: Option<i32>, stdout: &str, stderr: &str) -> ProcessOutput {
    ProcessOutput {
        exit,
        timed_out: false,
        stdout: stdout.into(),
        stderr: stderr.into(),
        truncated: false,
        elapsed: Duration::from_millis(100),
    }
}

impl FakeServices {
    pub fn with_program(mut self, name: &str, path: &str) -> Self {
        self.programs.insert(name.into(), path.into());
        self
    }

    pub fn with_process(
        mut self,
        f: impl Fn(&ProcessSpec) -> ProcessOutput + Send + Sync + 'static,
    ) -> Self {
        self.process = Box::new(f);
        self
    }

    /// The programs started so far.
    pub fn started(&self) -> Vec<ProcessSpec> {
        self.started.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolServices for FakeServices {
    fn env_var(&self, name: &str) -> Option<String> {
        self.env.get(name).cloned()
    }

    fn is_dir(&self, path: &str) -> bool {
        self.dirs.iter().any(|d| d == path)
    }

    /// Words split on whitespace; a line with shell syntax is refused.
    fn parse_command(&self, line: &str) -> Result<Vec<String>, String> {
        if line.contains(['|', ';', '&', '<', '>', '$', '`', '*']) {
            return Err("shell syntax".into());
        }
        let words: Vec<String> = line.split_whitespace().map(String::from).collect();
        if words.is_empty() {
            Err("empty command".into())
        } else {
            Ok(words)
        }
    }

    fn find_program(&self, name: &str) -> Option<PathBuf> {
        self.programs.get(name).cloned()
    }

    fn classify_command(&self, line: &str) -> CommandRisk {
        (self.classify)(line)
    }

    /// A rule covers a command when it is the command's words joined, or a prefix of them ending in `*`.
    fn command_rule_matches(&self, rule: &str, argv: &[String]) -> bool {
        let line = argv.join(" ");
        match rule.strip_suffix('*') {
            Some(prefix) => line.starts_with(prefix),
            None => rule == line,
        }
    }

    async fn run_process(&self, spec: &ProcessSpec) -> ProcessOutput {
        self.started.lock().unwrap().push(spec.clone());
        (self.process)(spec)
    }

    fn internal_limit(&self, limit: Duration) -> Duration {
        limit
    }

    fn vet_file(
        &self,
        arg: &str,
        _cwd: Option<&str>,
        _roots: &[String],
    ) -> Result<FileTarget, String> {
        (self.vet)(arg)
    }

    fn read_file(&self, target: &FileTarget, max_bytes: usize) -> Result<FileText, String> {
        (self.read)(target, max_bytes)
    }
}
