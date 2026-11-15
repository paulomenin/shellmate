//! Asking questions in `setup`. The flow talks to a `Prompter`, so a test can script the answers.

use shellmate_api::{Error, Result};
use std::io::{BufRead, Write};

pub trait Prompter {
    fn say(&mut self, text: &str);
    /// Ask a question. An empty answer gives `default`, which may be empty.
    fn ask(&mut self, question: &str, default: &str) -> String;
}

/// The terminal: questions on stdout, answers from stdin.
pub struct Terminal;

impl Prompter for Terminal {
    fn say(&mut self, text: &str) {
        println!("{text}");
    }

    fn ask(&mut self, question: &str, default: &str) -> String {
        if default.is_empty() {
            print!("{question} ");
        } else {
            print!("{question} [{default}] ");
        }
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).ok();
        match line.trim() {
            "" => default.to_string(),
            t => t.to_string(),
        }
    }
}

/// How many times a question is repeated before setup gives up, so a closed stdin cannot loop forever.
const ATTEMPTS: usize = 3;

fn no_answer(what: &str) -> Error {
    Error::new("setup.no_answer", format!("no valid answer for {what}"))
        .with_hint("run `shellmate setup` again, or pass the options with --yes")
}

/// Show a numbered list and ask for one. `default` is the item chosen by an empty answer, if any.
pub fn choose(
    p: &mut dyn Prompter,
    question: &str,
    items: &[String],
    default: Option<usize>,
) -> Result<usize> {
    for (i, item) in items.iter().enumerate() {
        p.say(&format!("  {}) {item}", i + 1));
    }
    let shown = default.map(|d| (d + 1).to_string()).unwrap_or_default();
    for _ in 0..ATTEMPTS {
        let answer = p.ask(question, &shown);
        match answer.trim().parse::<usize>() {
            Ok(n) if (1..=items.len()).contains(&n) => return Ok(n - 1),
            _ => p.say(&format!("enter a number from 1 to {}", items.len())),
        }
    }
    Err(no_answer(question))
}

/// Ask until something is typed. There is no default on purpose: the answer has to be the person's own.
pub fn ask_required(p: &mut dyn Prompter, question: &str) -> Result<String> {
    for _ in 0..ATTEMPTS {
        let answer = p.ask(question, "");
        if !answer.trim().is_empty() {
            return Ok(answer.trim().to_string());
        }
        p.say("this needs an answer");
    }
    Err(no_answer(question))
}

/// A yes or no. Only `y` or `yes` is a yes, so an empty answer is the default.
pub fn confirm(p: &mut dyn Prompter, question: &str, default: bool) -> bool {
    let shown = if default { "Y/n" } else { "y/N" };
    match p
        .ask(&format!("{question} [{shown}]"), "")
        .to_lowercase()
        .as_str()
    {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    }
}

/// Scripted answers for tests, in the order they are asked. What was said and asked is kept to look at.
#[cfg(test)]
pub struct Scripted {
    answers: std::collections::VecDeque<String>,
    pub said: Vec<String>,
    pub asked: Vec<String>,
}

#[cfg(test)]
impl Scripted {
    pub fn new(answers: &[&str]) -> Self {
        Self {
            answers: answers.iter().map(|a| a.to_string()).collect(),
            said: Vec::new(),
            asked: Vec::new(),
        }
    }

    /// Everything shown, for a test to search.
    pub fn output(&self) -> String {
        self.said.join("\n")
    }

    pub fn left(&self) -> usize {
        self.answers.len()
    }
}

#[cfg(test)]
impl Prompter for Scripted {
    fn say(&mut self, text: &str) {
        self.said.push(text.to_string());
    }

    fn ask(&mut self, question: &str, default: &str) -> String {
        self.asked.push(question.to_string());
        match self.answers.pop_front() {
            Some(a) if !a.trim().is_empty() => a,
            _ => default.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<String> {
        vec!["a".into(), "b".into(), "c".into()]
    }

    #[test]
    fn a_number_picks_an_item_and_an_empty_answer_takes_the_default() {
        let mut p = Scripted::new(&["2"]);
        assert_eq!(choose(&mut p, "pick", &items(), Some(0)).unwrap(), 1);
        let mut p = Scripted::new(&[""]);
        assert_eq!(choose(&mut p, "pick", &items(), Some(2)).unwrap(), 2);
        assert!(p.output().contains("1) a") && p.output().contains("3) c"));
    }

    #[test]
    fn a_bad_answer_is_asked_again_and_then_given_up_on() {
        let mut p = Scripted::new(&["9", "x", "3"]);
        assert_eq!(choose(&mut p, "pick", &items(), None).unwrap(), 2);
        assert_eq!(p.asked.len(), 3);
        // no default and no answer: an error, not a guess
        let mut p = Scripted::new(&[]);
        assert_eq!(
            choose(&mut p, "pick", &items(), None).unwrap_err().code,
            "setup.no_answer"
        );
    }

    #[test]
    fn a_required_answer_has_no_default() {
        let mut p = Scripted::new(&["", "  ", "model-x"]);
        assert_eq!(ask_required(&mut p, "model").unwrap(), "model-x");
        let mut p = Scripted::new(&[]);
        assert!(ask_required(&mut p, "model").is_err());
    }

    #[test]
    fn only_yes_is_a_yes() {
        for (answer, default, want) in [
            ("y", false, true),
            ("YES", false, true),
            ("", false, false),
            ("", true, true),
            ("n", true, false),
            ("maybe", false, false),
        ] {
            let mut p = Scripted::new(&[answer]);
            assert_eq!(confirm(&mut p, "ok?", default), want, "{answer:?}");
        }
    }
}
