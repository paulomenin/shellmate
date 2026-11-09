//! Looks: which style each kind of text gets, reduced to what the terminal can show.
//!
//! The palette is the 16 ANSI colours, so it follows the user's own terminal theme. Text is never given an
//! RGB colour. A role has one style at the richest level and is derived from it for the others.

use super::{Level, Tone};
use anstyle::{AnsiColor, Style};

/// What a piece of text is, independent of how it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Plain,
    Strong,
    Emph,
    StrongEmph,
    Code,
    CodeBlock,
    Heading,
    Quote,
    /// The marker in front of a list item, or the bar of a block.
    Marker,
    Rule,
    Muted,
    Info,
    Success,
    Warning,
    Error,
}

impl From<Tone> for Role {
    fn from(tone: Tone) -> Self {
        match tone {
            Tone::Info => Role::Info,
            Tone::Success => Role::Success,
            Tone::Warning => Role::Warning,
            Tone::Error => Role::Error,
            Tone::Muted => Role::Muted,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    level: Level,
}

impl Theme {
    pub fn new(level: Level) -> Self {
        Self { level }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    /// The style of `role` at this theme's level.
    pub fn style(&self, role: Role) -> Style {
        match self.level {
            Level::Plain => Style::new(),
            Level::Ansi16 => full(role),
            Level::Mono => mono(role),
        }
    }

    /// `text` in the style of `role`. `text` must already be safe to show: this adds the only escape
    /// sequences a presenter writes, and none that came from outside.
    pub fn paint(&self, role: Role, text: &str) -> String {
        let style = self.style(role);
        format!("{style}{text}{style:#}")
    }

    /// The waiting indicator cycles through three cool colours, a new one every few steps.
    pub fn spinner(&self, step: usize) -> Style {
        let colors = [AnsiColor::Cyan, AnsiColor::BrightBlue, AnsiColor::Magenta];
        match self.level {
            Level::Plain => Style::new(),
            Level::Mono => Style::new().bold(),
            Level::Ansi16 => colors[(step / 3) % colors.len()].on_default().bold(),
        }
    }
}

fn full(role: Role) -> Style {
    use AnsiColor::*;
    match role {
        Role::Plain | Role::CodeBlock => Style::new(),
        Role::Strong => Style::new().bold(),
        Role::Emph => Style::new().italic(),
        Role::StrongEmph => Style::new().bold().italic(),
        Role::Code => Magenta.on_default(),
        Role::Heading => Cyan.on_default().bold(),
        Role::Quote => Style::new().italic().dimmed(),
        Role::Marker => Cyan.on_default(),
        Role::Rule | Role::Muted => Style::new().dimmed(),
        Role::Info => Cyan.on_default(),
        Role::Success => Green.on_default().bold(),
        Role::Warning => Yellow.on_default(),
        Role::Error => Red.on_default().bold(),
    }
}

/// The same roles with no colour at all: only what `NO_COLOR` allows.
fn mono(role: Role) -> Style {
    match role {
        Role::Plain | Role::CodeBlock | Role::Marker | Role::Info | Role::Success => Style::new(),
        Role::Strong | Role::Warning | Role::Error => Style::new().bold(),
        Role::Emph => Style::new().italic(),
        Role::StrongEmph => Style::new().bold().italic(),
        Role::Code => Style::new().underline(),
        Role::Heading => Style::new().bold().underline(),
        Role::Quote => Style::new().italic(),
        Role::Rule | Role::Muted => Style::new().dimmed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Role; 15] = [
        Role::Plain,
        Role::Strong,
        Role::Emph,
        Role::StrongEmph,
        Role::Code,
        Role::CodeBlock,
        Role::Heading,
        Role::Quote,
        Role::Marker,
        Role::Rule,
        Role::Muted,
        Role::Info,
        Role::Success,
        Role::Warning,
        Role::Error,
    ];

    /// The numeric parameters of every SGR sequence in `s`.
    fn params(s: &str) -> Vec<u32> {
        s.split("\x1b[")
            .skip(1)
            .flat_map(|rest| {
                let end = rest.find('m').unwrap();
                rest[..end]
                    .split(';')
                    .map(|n| n.parse::<u32>().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn has_colour(s: &str) -> bool {
        params(s)
            .iter()
            .any(|p| (30..=37).contains(p) || (90..=97).contains(p))
    }

    #[test]
    fn plain_text_is_never_wrapped_in_escapes() {
        let t = Theme::new(Level::Plain);
        for role in ALL {
            assert_eq!(t.paint(role, "text"), "text", "{role:?}");
        }
        assert_eq!(t.paint(Role::Plain, "x"), "x");
    }

    #[test]
    fn plain_role_has_no_escapes_at_any_level() {
        for level in [Level::Plain, Level::Mono, Level::Ansi16] {
            assert_eq!(Theme::new(level).paint(Role::Plain, "x"), "x", "{level:?}");
        }
    }

    #[test]
    fn mono_uses_no_colour_anywhere() {
        let t = Theme::new(Level::Mono);
        for role in ALL {
            assert!(!has_colour(&t.paint(role, "x")), "{role:?}");
        }
        for step in 0..12 {
            assert!(!has_colour(&format!("{}x", t.spinner(step))));
        }
    }

    #[test]
    fn the_colours_carry_the_meaning_the_tones_promise() {
        let t = Theme::new(Level::Ansi16);
        let colour = |role| params(&t.paint(role, "x"));
        assert!(colour(Role::Error).contains(&31), "errors are red");
        assert!(colour(Role::Warning).contains(&33), "warnings are yellow");
        assert!(colour(Role::Success).contains(&32), "success is green");
        assert!(colour(Role::Info).contains(&36), "information is cyan");
        assert!(colour(Role::Muted).contains(&2), "background detail is dim");
    }

    #[test]
    fn every_styled_text_is_reset_afterwards() {
        for level in [Level::Mono, Level::Ansi16] {
            let t = Theme::new(level);
            for role in ALL {
                let out = t.paint(role, "x");
                if out != "x" {
                    assert!(
                        out.starts_with("\x1b[") && out.ends_with("\x1b[0m"),
                        "{role:?} {out:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_tone_maps_to_the_role_of_the_same_name() {
        assert_eq!(Role::from(Tone::Error), Role::Error);
        assert_eq!(Role::from(Tone::Muted), Role::Muted);
    }

    #[test]
    fn the_spinner_changes_colour_every_three_steps_and_cycles() {
        let t = Theme::new(Level::Ansi16);
        let c = |step| format!("{}", t.spinner(step));
        assert_eq!(c(0), c(2));
        assert_ne!(c(0), c(3));
        assert_eq!(c(0), c(9));
        assert_eq!(format!("{}", Theme::new(Level::Plain).spinner(5)), "");
    }
}
