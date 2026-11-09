//! A tool call as the model wrote it: one line, a name and its words.

/// A parsed tool call: the tool name and its argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub args: Vec<String>,
    /// Everything after the tool name, as written (the `exec` tool parses it with its own strict grammar).
    pub rest: String,
}

impl ToolCall {
    /// `man ssh` → name `man`, args `["ssh"]`. A block must be a single line.
    pub fn parse(body: &str) -> Result<Self, String> {
        let body = body.trim();
        if body.is_empty() || body.contains('\n') {
            return Err("a tool call must be a single line like `man tar`".into());
        }
        let mut words = body.split_whitespace().map(str::to_string);
        let name = words.next().expect("non-empty");
        let rest = body[name.len()..].trim().to_string();
        Ok(Self {
            name,
            args: words.collect(),
            rest,
        })
    }

    pub fn display(&self) -> String {
        std::iter::once(self.name.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}
