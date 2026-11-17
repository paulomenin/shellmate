//! Extract command suggestions from model text using the fenced convention
//! (`providers-prompts.md` §3, mechanism 3): each command is in a block labeled `shellmate-command`.

use crate::sanitize::is_buffer_safe;

pub const FENCE_LABEL: &str = "shellmate-command";
/// Blocks with this label are tool calls (`man ssh`), run by the engine and never shown.
pub const TOOL_LABEL: &str = shellmate_api::TOOL_LABEL;
/// The heading `ask` prints above the second and later command blocks. The prompt names it, so the model knows
/// that an extra block reads as an alternative, never as a next step.
pub const OTHER_OPTIONS_LABEL: &str = "Other options";

fn block_kind(line: &str) -> Option<&'static str> {
    let rest = line.trim_start().strip_prefix("```")?.trim();
    [FENCE_LABEL, TOOL_LABEL].into_iter().find(|l| *l == rest)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The model's text with command blocks removed.
    pub prose: String,
    /// Suggested commands in order; the first is the primary one.
    pub commands: Vec<String>,
    /// Tool calls requested, in order (the text inside each `shellmate-tool` block).
    pub tool_calls: Vec<String>,
    /// Commands dropped because they contained unsafe characters (reported, never placed in the buffer).
    pub rejected: usize,
}

pub fn parse(text: &str) -> Answer {
    let mut prose = String::new();
    let mut commands = Vec::new();
    let mut tool_calls = Vec::new();
    let mut rejected = 0;
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(kind) = block_kind(line) else {
            prose.push_str(line);
            prose.push('\n');
            continue;
        };
        let mut body = Vec::new();
        for l in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            body.push(l);
        }
        let cmd = body.join("\n").trim().to_string();
        if cmd.is_empty() {
            continue;
        }
        if kind == TOOL_LABEL {
            tool_calls.push(cmd);
        } else if cmd.chars().all(is_buffer_safe) {
            commands.push(cmd);
        } else {
            rejected += 1;
        }
    }
    Answer {
        prose: prose.trim().to_string(),
        commands,
        tool_calls,
        rejected,
    }
}

/// Streaming counterpart of [`parse`] for display: removes `shellmate-command` blocks from the text as it
/// arrives and passes everything else through with as little delay as possible. Only lines that could still
/// turn out to be a fence opener are held back until their newline.
#[derive(Default)]
pub struct DisplayFilter {
    line: String,
    /// The current line has been decided to be ordinary text and is already (partly) emitted.
    emitting: bool,
    in_fence: bool,
}

fn is_fence_opener(line: &str) -> bool {
    block_kind(line).is_some()
}

impl DisplayFilter {
    pub fn push(&mut self, delta: &str) -> String {
        let mut out = String::new();
        for ch in delta.chars() {
            if ch == '\n' {
                self.end_line(&mut out, true);
                continue;
            }
            self.line.push(ch);
            if self.in_fence {
                continue;
            }
            if self.emitting {
                out.push(ch);
                continue;
            }
            // line start: hold while it could still become a fence line
            let t = self.line.trim_start();
            let could_be_fence = t.is_empty() || t.starts_with("```") || "```".starts_with(t);
            if !could_be_fence {
                out.push_str(&std::mem::take(&mut self.line));
                // keep the line text for fence detection at the newline, but it is already out
                self.emitting = true;
                self.line.clear();
                self.line.push('x'); // non-empty marker; content is irrelevant once emitting
            }
        }
        out
    }

    /// Call once when the stream ends.
    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        self.end_line(&mut out, false);
        out
    }

    fn end_line(&mut self, out: &mut String, newline: bool) {
        let line = std::mem::take(&mut self.line);
        if self.emitting {
            if newline {
                out.push('\n');
            }
        } else if self.in_fence {
            if line.trim_start().starts_with("```") {
                self.in_fence = false;
            }
        } else if is_fence_opener(&line) {
            self.in_fence = true;
        } else {
            out.push_str(&line);
            if newline {
                out.push('\n');
            }
        }
        self.emitting = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_commands_and_keeps_prose() {
        let a = parse("Use du:\n```shellmate-command\ndu -sh * | sort -h\n```\nor\n```shellmate-command\nls -S\n```\nDone.");
        assert_eq!(a.commands, ["du -sh * | sort -h", "ls -S"]);
        assert_eq!(a.prose, "Use du:\nor\nDone.");
    }

    #[test]
    fn tool_blocks_are_extracted_and_hidden() {
        let t = "Let me check.\n```shellmate-tool\nman ssh\n```\n";
        let a = parse(t);
        assert_eq!(a.tool_calls, ["man ssh"]);
        assert!(a.commands.is_empty());
        assert_eq!(a.prose, "Let me check.");
        for chunk in [1, 3, 1000] {
            assert_eq!(filtered(t, chunk).trim(), "Let me check.", "chunk={chunk}");
        }
    }

    #[test]
    fn other_fences_are_prose() {
        let a = parse("```sh\nrm -rf /\n```");
        assert!(a.commands.is_empty());
        assert!(a.prose.contains("rm -rf /"));
    }

    #[test]
    fn a_command_with_backslashes_is_kept() {
        let text =
            "```shellmate-command\nfind . -type f -printf '%s %p\\n' | sort -nr | head -1\n```\n\
                    ```shellmate-command\nfind . -name '*.log' -exec ls -l {} \\;\n```";
        let a = parse(text);
        assert_eq!(a.rejected, 0);
        assert_eq!(a.commands.len(), 2, "{:?}", a.commands);
        assert!(a.commands[0].contains("%p\\n"));
    }

    #[test]
    fn unsafe_characters_are_rejected_not_buffered() {
        let a = parse("```shellmate-command\necho hi\u{1b}[2J\n```\n```shellmate-command\nok\n```");
        assert_eq!(a.commands, ["ok"]);
        assert_eq!(a.rejected, 1);
    }

    #[test]
    fn unterminated_block_still_yields_the_command() {
        assert_eq!(parse("```shellmate-command\nls -la").commands, ["ls -la"]);
    }

    #[test]
    fn multiline_commands_keep_their_newlines() {
        assert_eq!(
            parse("```shellmate-command\nfor f in *; do\n  echo $f\ndone\n```").commands,
            ["for f in *; do\n  echo $f\ndone"]
        );
    }

    fn filtered(text: &str, chunk: usize) -> String {
        let mut f = DisplayFilter::default();
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        for c in chars.chunks(chunk) {
            out.push_str(&f.push(&c.iter().collect::<String>()));
        }
        out.push_str(&f.finish());
        out
    }

    #[test]
    fn display_filter_matches_parse_for_any_chunking() {
        let texts = [
            "Use du:\n```shellmate-command\ndu -sh * | sort -h\n```\nor\n```shellmate-command\nls -S\n```\nDone.",
            "no commands here\nsecond line",
            "```sh\nrm -rf /\n```\nafter",
            "intro\n  ```shellmate-command\n  indented\n  ```\nend",
            "```shellmate-command\nunterminated",
            "a ``` inline fence and `code`\n",
            "",
        ];
        for t in texts {
            let want = parse(t).prose;
            for chunk in [1, 2, 3, 7, 1000] {
                assert_eq!(filtered(t, chunk).trim(), want, "chunk={chunk} text={t:?}");
            }
        }
    }

    #[test]
    fn display_filter_does_not_delay_ordinary_text() {
        let mut f = DisplayFilter::default();
        assert_eq!(f.push("Hel"), "Hel");
        assert_eq!(f.push("lo wor"), "lo wor");
        assert_eq!(f.push("ld\nne"), "ld\nne");
        // a line that might become a fence is held until it is decided
        assert_eq!(f.push("\n``"), "\n");
        assert_eq!(f.push("`sh\n"), "```sh\n");
    }
}
