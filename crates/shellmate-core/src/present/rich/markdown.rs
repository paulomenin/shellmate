//! A streaming reader for the small part of markdown a terminal answer needs.
//!
//! Supported: `#` headings, `-`/`*`/`+` and numbered lists (nested by indentation), `>` quotes, fenced code
//! blocks, `---` rules, and inline `**strong**`, `*emphasis*`, `_emphasis_` and `` `code` ``. Anything else
//! is shown as written. The reader drives a [`Wrap`], which escapes and wraps; nothing here writes bytes.
//!
//! It reads one character at a time with explicit state, so the result cannot depend on how the text was
//! split into pieces. It never redraws: a line's start is held back only while its kind is unclear (a few
//! characters, and only after `#`, `-`, `*`, `+`, `>`, a digit or a backtick), and an inline span is held back
//! only until its closing delimiter or the end of the line. A span that never closes is shown literally.

use super::wrap::Wrap;
use crate::present::Role;

/// Most characters held back while a span is open; a longer one is shown literally.
const MAX_HELD: usize = 4000;

pub(super) struct Markdown {
    mode: Mode,
    /// Characters of the current line held back while its kind is unclear.
    buf: String,
    inline: Inline,
    /// The role of the text of the current line.
    base: Role,
    /// A list item or quote the next unmarked line still belongs to (with the role of its text).
    cont: Option<Role>,
    fence: Option<(char, usize)>,
    unicode: bool,
    width: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// At the start of a line, deciding what it is.
    LineStart,
    /// Inside a line of ordinary text.
    Inline,
    /// The rest of the line that opened a code block (the language), which is not shown.
    FenceInfo,
    /// At the start of a line inside a code block, which might close it.
    CodeStart,
    /// Inside a line of code.
    Code,
}

impl Markdown {
    pub(super) fn new(width: usize, unicode: bool) -> Self {
        Self {
            mode: Mode::LineStart,
            buf: String::new(),
            inline: Inline::default(),
            base: Role::Plain,
            cont: None,
            fence: None,
            unicode,
            width,
        }
    }

    pub(super) fn push(&mut self, text: &str, wrap: &mut Wrap) {
        for c in text.chars() {
            self.char(c, wrap);
        }
    }

    /// The end of the text: settle whatever is still undecided.
    pub(super) fn finish(&mut self, wrap: &mut Wrap) {
        self.settle(wrap);
        wrap.finish();
    }

    /// Something other than the model's text is about to be shown (a tool notice): end the line being read
    /// so that it starts on a line of its own, and forget any block.
    pub(super) fn interrupt(&mut self, wrap: &mut Wrap) {
        self.settle(wrap);
        if !wrap.at_line_start() {
            wrap.newline();
        }
        wrap.end_block();
        self.cont = None;
        self.fence = None;
        self.mode = Mode::LineStart;
    }

    fn settle(&mut self, wrap: &mut Wrap) {
        match self.mode {
            Mode::LineStart => {
                if self.is_rule() {
                    self.rule(wrap);
                } else if !self.buf.trim().is_empty() {
                    self.paragraph(wrap);
                    self.end_inline(wrap);
                }
                self.buf.clear();
            }
            Mode::Inline => {
                self.end_inline(wrap);
                wrap.open_line();
            }
            Mode::CodeStart => {
                if self.closes_fence() {
                    self.close_fence(wrap);
                } else {
                    let held = std::mem::take(&mut self.buf);
                    wrap.raw(&held, Role::CodeBlock);
                }
            }
            Mode::FenceInfo | Mode::Code => {}
        }
    }

    fn end_inline(&mut self, wrap: &mut Wrap) {
        self.inline.end_line(self.base, wrap);
    }

    fn char(&mut self, c: char, wrap: &mut Wrap) {
        match self.mode {
            Mode::LineStart => {
                if c == '\n' {
                    self.start_newline(wrap);
                } else {
                    self.buf.push(c);
                    self.classify(wrap);
                }
            }
            Mode::Inline => {
                if c == '\n' {
                    self.end_inline(wrap);
                    wrap.open_line();
                    wrap.newline();
                    self.mode = Mode::LineStart;
                } else {
                    self.inline.push(c, self.base, wrap);
                }
            }
            Mode::FenceInfo => {
                if c == '\n' {
                    self.mode = Mode::CodeStart;
                }
            }
            Mode::CodeStart => {
                if c == '\n' {
                    self.code_start_newline(wrap);
                } else {
                    self.buf.push(c);
                    self.classify_code(wrap);
                }
            }
            Mode::Code => {
                if c == '\n' {
                    wrap.newline();
                    self.mode = Mode::CodeStart;
                } else {
                    wrap.raw(&c.to_string(), Role::CodeBlock);
                }
            }
        }
    }

    // ---- the start of a line -------------------------------------------------------------------------

    /// A line ended while its kind was still unclear.
    fn start_newline(&mut self, wrap: &mut Wrap) {
        let held: Vec<char> = self.buf.chars().collect();
        let rest: Vec<char> = held.iter().copied().skip_while(|c| *c == ' ').collect();
        let c0 = rest.first().copied().unwrap_or(' ');
        if rest.is_empty() {
            // a blank line ends any list or quote
            self.buf.clear();
            self.cont = None;
            wrap.end_block();
            wrap.newline();
            return;
        }
        if self.is_rule() {
            self.rule(wrap);
            return;
        }
        if matches!(c0, '`' | '~') && rest.len() >= 3 && rest.iter().all(|c| *c == c0) {
            // a fence with no language: the line ends right after it
            self.open_fence(c0, rest.len(), wrap);
            self.mode = Mode::CodeStart;
            return;
        }
        self.paragraph(wrap);
        self.end_inline(wrap);
        wrap.newline();
        self.mode = Mode::LineStart;
    }

    /// The held line is only `---`, `***` or `___` (three or more, spaces allowed).
    fn is_rule(&self) -> bool {
        let rest: Vec<char> = self.buf.chars().skip_while(|c| *c == ' ').collect();
        let Some(&c0) = rest.first() else {
            return false;
        };
        matches!(c0, '-' | '*' | '_')
            && rest.iter().all(|c| *c == c0 || *c == ' ')
            && rest.iter().filter(|c| **c == c0).count() >= 3
    }

    fn rule(&mut self, wrap: &mut Wrap) {
        self.buf.clear();
        self.cont = None;
        wrap.end_block();
        let bar = if self.unicode { "─" } else { "-" };
        wrap.text(&bar.repeat(self.width.min(40)), Role::Rule);
        wrap.newline();
    }

    fn classify(&mut self, wrap: &mut Wrap) {
        let held: Vec<char> = self.buf.chars().collect();
        let indent = held.iter().take_while(|c| **c == ' ').count();
        let rest = &held[indent..];
        let Some(&c0) = rest.first() else {
            return; // only spaces so far
        };
        let run = rest.iter().take_while(|c| **c == c0).count();
        match c0 {
            '#' => {
                if run > 6 {
                    return self.paragraph(wrap);
                }
                match rest.get(run) {
                    None => {}
                    Some(' ') => self.heading(wrap),
                    Some(_) => self.paragraph(wrap),
                }
            }
            '-' | '+' | '*' | '_' => match rest.get(1) {
                None => {}
                Some(' ') if c0 != '_' => self.bullet(indent, wrap),
                // maybe a rule (`---`) or strong text (`**x**`): the whole line decides
                Some(_) if !rest.iter().all(|c| *c == c0 || *c == ' ') => self.paragraph(wrap),
                Some(_) => {}
            },
            d if d.is_ascii_digit() => {
                let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
                match rest.get(digits) {
                    None => {}
                    Some('.') | Some(')') => match rest.get(digits + 1) {
                        None => {}
                        Some(' ') if digits <= 9 => {
                            let number: String = rest[..=digits].iter().collect();
                            self.ordered(indent, &number, wrap);
                        }
                        Some(_) => self.paragraph(wrap),
                    },
                    Some(_) => self.paragraph(wrap),
                }
            }
            '>' => self.quote(wrap),
            '`' | '~' => match rest.get(run) {
                None => {}
                Some(_) if run >= 3 => self.open_fence(c0, run, wrap),
                Some(_) => self.paragraph(wrap),
            },
            _ => self.paragraph(wrap),
        }
    }

    /// The line is ordinary text: give what was held back to the inline reader.
    fn paragraph(&mut self, wrap: &mut Wrap) {
        match self.cont {
            Some(role) => self.base = role,
            None => {
                wrap.end_block();
                self.base = Role::Plain;
            }
        }
        self.mode = Mode::Inline;
        let held = std::mem::take(&mut self.buf);
        for c in held.chars() {
            self.inline.push(c, self.base, wrap);
        }
    }

    fn heading(&mut self, wrap: &mut Wrap) {
        wrap.end_block();
        self.cont = None;
        self.base = Role::Heading;
        self.buf.clear();
        self.mode = Mode::Inline;
    }

    fn bullet(&mut self, indent: usize, wrap: &mut Wrap) {
        let indent = indent.min(8);
        let marker = if self.unicode { "•" } else { "-" };
        let first = format!("{}{marker} ", " ".repeat(indent));
        let rest = " ".repeat(indent + 2);
        wrap.block((&first, Role::Marker), (&rest, Role::Plain));
        self.start_item();
    }

    fn ordered(&mut self, indent: usize, number: &str, wrap: &mut Wrap) {
        let indent = indent.min(8);
        let first = format!("{}{number} ", " ".repeat(indent));
        let rest = " ".repeat(first.chars().count());
        wrap.block((&first, Role::Marker), (&rest, Role::Plain));
        self.start_item();
    }

    fn start_item(&mut self) {
        self.cont = Some(Role::Plain);
        self.base = Role::Plain;
        self.buf.clear();
        self.mode = Mode::Inline;
    }

    fn quote(&mut self, wrap: &mut Wrap) {
        let bar = if self.unicode { "▎ " } else { "| " };
        wrap.block((bar, Role::Marker), (bar, Role::Marker));
        self.cont = Some(Role::Quote);
        self.base = Role::Quote;
        // the marker and the space after it are not text
        self.buf.clear();
        self.mode = Mode::Inline;
    }

    fn open_fence(&mut self, ch: char, n: usize, wrap: &mut Wrap) {
        let bar = if self.unicode { "│ " } else { "| " };
        wrap.end_block();
        wrap.block((bar, Role::Rule), (bar, Role::Rule));
        self.cont = None;
        self.fence = Some((ch, n));
        self.buf.clear();
        self.mode = Mode::FenceInfo;
    }

    // ---- inside a code block ------------------------------------------------------------------------

    /// A line of the code block ended while it might still be the closing fence.
    fn code_start_newline(&mut self, wrap: &mut Wrap) {
        if self.closes_fence() {
            self.close_fence(wrap);
        } else {
            let held = std::mem::take(&mut self.buf);
            wrap.raw(&held, Role::CodeBlock);
            wrap.newline();
            self.mode = Mode::CodeStart;
        }
    }

    fn classify_code(&mut self, wrap: &mut Wrap) {
        let Some((ch, _)) = self.fence else {
            return;
        };
        let held: Vec<char> = self.buf.chars().collect();
        let indent = held.iter().take_while(|c| **c == ' ').count();
        let rest = &held[indent..];
        let looks_like_a_fence = indent <= 3 && rest.iter().all(|c| *c == ch || *c == ' ');
        if !looks_like_a_fence {
            let held = std::mem::take(&mut self.buf);
            wrap.raw(&held, Role::CodeBlock);
            self.mode = Mode::Code;
        }
    }

    fn closes_fence(&self) -> bool {
        let Some((ch, n)) = self.fence else {
            return false;
        };
        let held: Vec<char> = self.buf.chars().collect();
        let indent = held.iter().take_while(|c| **c == ' ').count();
        let rest = &held[indent..];
        indent <= 3
            && rest.iter().filter(|c| **c == ch).count() >= n
            && rest.iter().all(|c| *c == ch || *c == ' ')
    }

    fn close_fence(&mut self, wrap: &mut Wrap) {
        self.buf.clear();
        self.fence = None;
        wrap.end_block();
        self.mode = Mode::LineStart;
    }
}

// ---- inline spans -----------------------------------------------------------------------------------

#[derive(Default)]
struct Inline {
    frames: Vec<Frame>,
    /// Finished nodes while a span is open further out.
    root: Vec<Node>,
    run: Option<Run>,
    /// The last character before the run being read.
    prev: Option<char>,
    held: usize,
}

struct Run {
    ch: char,
    n: usize,
    prev: Option<char>,
}

struct Frame {
    kind: Kind,
    ch: char,
    n: usize,
    children: Vec<Node>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Emph,
    Strong,
    Both,
    Code,
}

enum Node {
    Text(String),
    Span(Kind, Vec<Node>),
}

impl Inline {
    fn push(&mut self, c: char, base: Role, wrap: &mut Wrap) {
        if matches!(c, '*' | '_' | '`') {
            match &mut self.run {
                Some(run) if run.ch == c => run.n += 1,
                _ => {
                    self.resolve(Some(c), base, wrap);
                    self.run = Some(Run {
                        ch: c,
                        n: 1,
                        prev: self.prev,
                    });
                }
            }
            return;
        }
        self.resolve(Some(c), base, wrap);
        self.text(&c.to_string(), base, wrap);
        self.prev = Some(c);
    }

    /// The end of the line: nothing that is still open will close, so show it as written.
    fn end_line(&mut self, base: Role, wrap: &mut Wrap) {
        self.resolve(None, base, wrap);
        self.unwind(base, wrap);
        self.prev = None;
    }

    fn text(&mut self, s: &str, base: Role, wrap: &mut Wrap) {
        match self.frames.last_mut() {
            None => wrap.text(s, base),
            Some(frame) => {
                self.held += s.chars().count();
                match frame.children.last_mut() {
                    Some(Node::Text(t)) => t.push_str(s),
                    _ => frame.children.push(Node::Text(s.to_string())),
                }
                if self.held > MAX_HELD {
                    self.unwind(base, wrap);
                }
            }
        }
    }

    /// A run of delimiters ended; `next` is the character after it.
    fn resolve(&mut self, next: Option<char>, base: Role, wrap: &mut Wrap) {
        let Some(Run { ch, n, prev }) = self.run.take() else {
            return;
        };
        self.prev = Some(ch);
        let literal = ch.to_string().repeat(n);
        let in_code = self.frames.last().is_some_and(|f| f.kind == Kind::Code);
        if ch == '`' {
            if in_code {
                if self.frames.last().is_some_and(|f| f.n == n) {
                    return self.close(base, wrap);
                }
                return self.text(&literal, base, wrap);
            }
            return self.open(Kind::Code, ch, n);
        }
        if in_code {
            return self.text(&literal, base, wrap);
        }
        let kind = match n {
            1 => Kind::Emph,
            2 => Kind::Strong,
            3 => Kind::Both,
            _ => return self.text(&literal, base, wrap),
        };
        let can_close = self
            .frames
            .last()
            .is_some_and(|f| f.ch == ch && f.n == n && !f.children.is_empty())
            && prev.is_some_and(|p| !p.is_whitespace())
            && (ch == '*' || next.is_none_or(|x| !x.is_alphanumeric()));
        if can_close {
            return self.close(base, wrap);
        }
        let can_open = next.is_some_and(|x| !x.is_whitespace())
            && (ch == '*' || prev.is_none_or(|p| !p.is_alphanumeric()));
        if can_open {
            self.open(kind, ch, n);
        } else {
            self.text(&literal, base, wrap);
        }
    }

    fn open(&mut self, kind: Kind, ch: char, n: usize) {
        self.frames.push(Frame {
            kind,
            ch,
            n,
            children: Vec::new(),
        });
    }

    fn close(&mut self, base: Role, wrap: &mut Wrap) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        let node = Node::Span(frame.kind, frame.children);
        match self.frames.last_mut() {
            Some(parent) => parent.children.push(node),
            None => {
                self.held = 0;
                let mut roots = std::mem::take(&mut self.root);
                roots.push(node);
                emit(&roots, Flags::default(), base, wrap);
            }
        }
    }

    /// Show every open span as the characters it was written with, then everything finished.
    fn unwind(&mut self, base: Role, wrap: &mut Wrap) {
        while let Some(frame) = self.frames.pop() {
            let mut nodes = vec![Node::Text(frame.ch.to_string().repeat(frame.n))];
            nodes.extend(frame.children);
            match self.frames.last_mut() {
                Some(parent) => parent.children.extend(nodes),
                None => self.root.extend(nodes),
            }
        }
        self.held = 0;
        let roots = std::mem::take(&mut self.root);
        emit(&roots, Flags::default(), base, wrap);
    }
}

#[derive(Clone, Copy, Default)]
struct Flags {
    strong: bool,
    emph: bool,
    code: bool,
}

fn emit(nodes: &[Node], flags: Flags, base: Role, wrap: &mut Wrap) {
    for node in nodes {
        match node {
            Node::Text(t) => wrap.text(t, role_for(flags, base)),
            Node::Span(kind, children) => {
                let mut inner = flags;
                match kind {
                    Kind::Strong => inner.strong = true,
                    Kind::Emph => inner.emph = true,
                    Kind::Both => {
                        inner.strong = true;
                        inner.emph = true;
                    }
                    Kind::Code => inner.code = true,
                }
                emit(children, inner, base, wrap);
            }
        }
    }
}

/// Code always looks like code. Strong and emphasis change plain text; in a heading or a quote the line's
/// own look stays.
fn role_for(flags: Flags, base: Role) -> Role {
    if flags.code {
        return Role::Code;
    }
    if base != Role::Plain {
        return base;
    }
    match (flags.strong, flags.emph) {
        (true, true) => Role::StrongEmph,
        (true, false) => Role::Strong,
        (false, true) => Role::Emph,
        (false, false) => Role::Plain,
    }
}

#[cfg(test)]
mod tests;
