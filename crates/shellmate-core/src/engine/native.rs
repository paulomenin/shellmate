//! Native tool calling: how one model is offered the tools, and what it sends back.
//!
//! A model gets its tools in one of two ways. In **text** mode the prompt describes them and the model writes
//! a `shellmate-tool` block. In **native** mode the provider is sent the tools (`ChatRequest::tools`) and the
//! model answers with calls in the provider's own protocol. The config says which (`tool_calling`), and
//! `auto`, the default, asks the provider whether the model takes tools natively.
//!
//! Either way a call ends up as the same one-line text, so the strict parsers, the classifier, approval and
//! taint are the same for both: a native call is only a different way to write the line.
//!
//! The answer to "does this model take tools?" is remembered in a small file, because a request is a new
//! process and asking the server on every request would cost a round trip each time.

use super::{Action, Engine, ProviderSpec};
use crate::config::ToolCalling;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shellmate_api::{ToolRequest, ToolSupport};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// How long a "yes" or a "no" is believed. A model's capabilities change with a new release of the server,
/// not from one day to the next.
const KNOWN_FOR_SECS: u64 = 7 * 24 * 3600;

/// How long "the server does not say" is believed. It is remembered too, because a server that never says
/// would otherwise be asked on every request, and every request is a new process. It is shorter, because
/// the answer to the same question may change when the server is updated.
const UNKNOWN_FOR_SECS: u64 = 3600;

/// The longest wait for a server to say whether a model takes tools. Past it the answer is "not known" and
/// the request goes ahead in text mode; nothing is remembered, so the next request asks again.
const ASK_FOR: Duration = Duration::from_millis(2000);

/// The most arguments a native call may carry. Longer is not a call anyone needs to make.
const MAX_ARGUMENT_BYTES: usize = 8 * 1024;

/// The most models remembered. The oldest go first.
const MAX_REMEMBERED: usize = 64;

/// How one request offers its tools to one model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ToolMode {
    pub(super) native: bool,
    /// The mode was worked out from what the server said, not set in the config. Only such a choice is
    /// taken back when the server refuses it.
    pub(super) learned: bool,
}

impl ToolMode {
    pub(super) const TEXT: ToolMode = ToolMode {
        native: false,
        learned: false,
    };
}

#[derive(Serialize, Deserialize, Default)]
struct Remembered {
    #[serde(default)]
    entries: BTreeMap<String, Entry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    /// `None`: the server was asked and did not say.
    native: Option<bool>,
    at: u64,
}

impl Engine {
    /// How `spec` is offered the tools of `action`. In `auto` mode, and only when nothing fresh is
    /// remembered, `ask` lets it ask the server; without it the answer is what was remembered, or text (for
    /// `--dry-run`, which sends nothing). Never fails: when in doubt the answer is text, which every model can
    /// do.
    pub(super) async fn tool_mode(
        &self,
        spec: &ProviderSpec,
        action: Action,
        ask: bool,
    ) -> ToolMode {
        if !self.offers_natively(action) {
            return ToolMode::TEXT;
        }
        match spec.tool_calling {
            ToolCalling::Text => ToolMode::TEXT,
            ToolCalling::Native => ToolMode {
                native: true,
                learned: false,
            },
            ToolCalling::Auto => ToolMode {
                native: self.model_takes_tools(spec, ask).await,
                learned: true,
            },
        }
    }

    /// Whether some tool has a native form for `action`. Without one there is nothing to send natively.
    fn offers_natively(&self, action: Action) -> bool {
        !super::tool_calls::specs(&self.plugged, action, &self.host).is_empty()
    }

    /// What is known about the model: remembered, or (when `ask` is set) asked of the server and remembered.
    async fn model_takes_tools(&self, spec: &ProviderSpec, ask: bool) -> bool {
        let key = Self::model_key(spec);
        if let Some(known) = self.recall(&key) {
            return known.unwrap_or(false);
        }
        if !ask {
            return false;
        }
        let said =
            tokio::time::timeout(crate::limits::scaled(ASK_FOR), spec.provider.tool_support())
                .await
                .unwrap_or(ToolSupport::Unknown);
        let native = match said {
            ToolSupport::Yes => Some(true),
            ToolSupport::No => Some(false),
            ToolSupport::Unknown => None,
        };
        self.remember(&key, native);
        native.unwrap_or(false)
    }

    /// The server refused a request that had tools in it: this model does not take them. Remember it, so
    /// the next request goes out in text mode at once.
    pub(super) fn forget_native(&self, spec: &ProviderSpec) {
        self.remember(&Self::model_key(spec), Some(false));
    }

    /// What identifies a model for this purpose: the plugin, where it is reached and its name.
    fn model_key(spec: &ProviderSpec) -> String {
        let info = spec.provider.info();
        format!(
            "{}|{}|{}",
            info.plugin,
            info.endpoint.unwrap_or_default(),
            info.model
        )
    }

    /// Where the answers are kept. `None` when there is no cache directory to keep them in.
    fn remembered_path(&self) -> Option<PathBuf> {
        let dir = self
            .host
            .env
            .nonempty("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| self.home().map(|h| PathBuf::from(h).join(".cache")))?;
        Some(dir.join("shellmate/tool-support.json"))
    }

    fn read_remembered(&self) -> Remembered {
        self.remembered_path()
            .and_then(|p| self.host.fs.read_to_string(&p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// `Some(answer)` when the model was asked about recently enough (the answer may be "not known").
    fn recall(&self, key: &str) -> Option<Option<bool>> {
        let now = self.host.clock.unix_secs();
        self.read_remembered()
            .entries
            .get(key)
            .filter(|e| {
                let valid_for = if e.native.is_some() {
                    KNOWN_FOR_SECS
                } else {
                    UNKNOWN_FOR_SECS
                };
                now.saturating_sub(e.at) < valid_for
            })
            .map(|e| e.native)
    }

    /// Best effort: a cache that cannot be written only means the server is asked again next time.
    fn remember(&self, key: &str, native: Option<bool>) {
        let Some(path) = self.remembered_path() else {
            return;
        };
        let now = self.host.clock.unix_secs();
        let mut all = self.read_remembered();
        all.entries
            .insert(key.to_string(), Entry { native, at: now });
        while all.entries.len() > MAX_REMEMBERED {
            let oldest = all
                .entries
                .iter()
                .min_by_key(|(_, e)| e.at)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(k) => all.entries.remove(&k),
                None => break,
            };
        }
        let Ok(text) = serde_json::to_string(&all) else {
            return;
        };
        let _ = write_private(&path, &text);
    }

    /// The text line of a native call: `man tar`, from the tool's name and its JSON arguments. The tool
    /// only reads the arguments; the line then goes through the same parser and checks as a call written in
    /// text. `Err` says why the arguments are not a call, for the model to read.
    pub(super) fn native_line(&self, call: &ToolRequest) -> Result<String, String> {
        let Some(tool) = self.plugged.iter().find(|p| p.name() == call.name) else {
            return Err(format!("there is no tool called `{}`", call.name));
        };
        if call.arguments.len() > MAX_ARGUMENT_BYTES {
            return Err(format!(
                "the arguments are too long ({} bytes, at most {MAX_ARGUMENT_BYTES})",
                call.arguments.len()
            ));
        }
        let arguments: Value = serde_json::from_str(&call.arguments)
            .map_err(|e| format!("the arguments are not valid JSON ({e})"))?;
        let line = tool.call_line(&arguments)?;
        Ok(format!("{} {line}", call.name))
    }
}

/// Write `text` to `path` for the user only, through a temporary file so a reader never sees half of it.
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(text.as_bytes())?;
    drop(file);
    std::fs::rename(&tmp, path)
}
