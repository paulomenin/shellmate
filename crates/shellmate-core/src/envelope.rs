//! The glue → binary envelope (`glue.md` §2): NUL-terminated `key=value` fields on stdin;
//! `buffer` is the last field and may contain newlines.

use shellmate_api::{Error, Result};

pub const MAX_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Envelope {
    pub version: u32,
    pub action: String,
    pub shell: String,
    pub shell_version: Option<String>,
    pub cwd: Option<String>,
    /// Cursor in Unicode scalar values.
    pub cursor: Option<usize>,
    pub status: Option<i32>,
    pub last_command: Option<String>,
    pub duration_ms: Option<u64>,
    pub hist: Vec<(i32, String)>,
    /// The interactive shell's session id (`SHELLMATE_SESSION`); `None` means stateless.
    pub session: Option<String>,
    pub buffer: String,
}

impl Envelope {
    pub fn parse(input: &[u8]) -> Result<Self> {
        let mut env = Envelope::default();
        for field in input.split(|b| *b == 0).filter(|f| !f.is_empty()) {
            let field = String::from_utf8_lossy(field);
            let Some((key, value)) = field.split_once('=') else {
                return Err(Error::new(
                    "glue.protocol",
                    format!(
                        "malformed envelope field `{}`",
                        crate::sanitize::escape(&field.chars().take(40).collect::<String>())
                    ),
                ));
            };
            match key {
                "v" => {
                    env.version = value
                        .parse()
                        .map_err(|_| Error::new("glue.protocol", "bad version"))?
                }
                "action" => env.action = value.to_string(),
                "shell" => env.shell = value.to_string(),
                "shell_version" => env.shell_version = Some(value.to_string()),
                "cwd" => env.cwd = Some(value.to_string()),
                "cursor" => env.cursor = value.parse().ok(),
                "status" => env.status = value.parse().ok(),
                "last_command" => env.last_command = Some(value.to_string()),
                "duration_ms" => env.duration_ms = value.parse().ok(),
                "hist" => {
                    if let Some((st, cmd)) = value.split_once('\t') {
                        env.hist.push((st.parse().unwrap_or(0), cmd.to_string()));
                    }
                }
                "session" if !value.is_empty() => env.session = Some(value.to_string()),
                "buffer" => env.buffer = value.to_string(),
                _ => {} // unknown fields are ignored (forward compatibility)
            }
        }
        if env.version != 1 {
            return Err(Error::new(
                "glue.version",
                format!("unsupported glue protocol version {}", env.version),
            )
            .with_hint("regenerate the shell integration: `shellmate init <shell>`"));
        }
        if env.action.is_empty() {
            return Err(Error::new("glue.protocol", "envelope has no `action`"));
        }
        if env.buffer.len() > MAX_BUFFER_BYTES {
            return Err(Error::new(
                "input.too_large",
                format!(
                    "the command line is larger than {} KiB",
                    MAX_BUFFER_BYTES / 1024
                ),
            ));
        }
        Ok(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields_and_multiline_buffer() {
        let raw = b"v=1\0action=ask\0shell=fish\0cwd=/tmp\0cursor=3\0status=2\0hist=0\tls -la\0hist=1\tgit push\0buffer=line1\nline2 = x\0";
        let e = Envelope::parse(raw).unwrap();
        assert_eq!(
            (e.action.as_str(), e.shell.as_str(), e.cursor, e.status),
            ("ask", "fish", Some(3), Some(2))
        );
        assert_eq!(
            e.hist,
            [(0, "ls -la".to_string()), (1, "git push".to_string())]
        );
        assert_eq!(e.buffer, "line1\nline2 = x");
    }

    #[test]
    fn rejects_bad_versions_and_oversized_buffers() {
        assert_eq!(
            Envelope::parse(b"v=2\0action=ask\0").unwrap_err().code,
            "glue.version"
        );
        assert_eq!(
            Envelope::parse(b"action=ask\0").unwrap_err().code,
            "glue.version"
        );
        let mut big = b"v=1\0action=ask\0buffer=".to_vec();
        big.resize(big.len() + MAX_BUFFER_BYTES + 1, b'x');
        assert_eq!(Envelope::parse(&big).unwrap_err().code, "input.too_large");
    }

    #[test]
    fn unknown_fields_are_ignored_and_garbage_is_rejected() {
        assert!(Envelope::parse(b"v=1\0action=ask\0future=1\0").is_ok());
        assert_eq!(
            Envelope::parse(b"v=1\0nonsense\0").unwrap_err().code,
            "glue.protocol"
        );
    }
}
