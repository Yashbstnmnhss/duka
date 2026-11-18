use std::borrow::Cow;

use colored::Colorize;
use rustyline::{
    Helper,
    completion::{Candidate, Completer},
    highlight::Highlighter as RlHighlighter,
    hint::{Hint as RlHint, Hinter},
    validate::Validator,
};

pub const COMMANDS: &[&str] = &[
    "break", "delete", "breaks", "continue", "next", "step", "out", "finish", "frames", "frame",
    "params", "args", "locals", "list", "quit", "help",
];

pub struct DbgHelper;

impl Helper for DbgHelper {}
impl Validator for DbgHelper {}

pub struct DukaDbgHint {
    pub display: String,
    pub completion: Option<String>,
}
impl RlHint for DukaDbgHint {
    fn display(&self) -> &str {
        &self.display
    }
    fn completion(&self) -> Option<&str> {
        self.completion.as_deref()
    }
}

impl Hinter for DbgHelper {
    type Hint = DukaDbgHint;

    fn hint(&self, line: &str, pos: usize, _: &rustyline::Context<'_>) -> Option<Self::Hint> {
        if !line.is_empty() && pos == line.len() {
            let prefix = line;
            for cmd in COMMANDS {
                if cmd.starts_with(prefix) && cmd.len() > prefix.len() {
                    let suffix = cmd[prefix.len()..].to_string();
                    return Some(DukaDbgHint {
                        display: suffix,
                        completion: Some(format!("?{}", cmd)),
                    });
                }
            }
        }
        None
    }
}

impl Completer for DbgHelper {
    type Candidate = CommandCandidate;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let prefix = &line[..pos];
        if prefix.contains(char::is_whitespace) {
            return Ok((pos, vec![]));
        }
        let matches = COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(prefix))
            .map(|cmd| CommandCandidate((*cmd).to_owned()))
            .collect();
        Ok((0, matches))
    }
}

pub struct CommandCandidate(String);
impl Candidate for CommandCandidate {
    fn display(&self) -> &str {
        &self.0
    }
    fn replacement(&self) -> &str {
        &self.0
    }
}

impl RlHighlighter for DbgHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Owned(prompt.bright_cyan().to_string())
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(hint.bright_white().to_string())
    }
}
