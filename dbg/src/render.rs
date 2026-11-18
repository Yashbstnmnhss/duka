use std::fmt::Write;

use colored::Colorize;
use duka_lib::value::RuntimeValue;
use duka_lib::vm::debugger::DebugFrameInfo;
use syntect::{
    easy::HighlightLines,
    highlighting::{Style, Theme, ThemeSet},
    parsing::{SyntaxDefinition, SyntaxSet},
    util::LinesWithEndings,
};

const DUKA_SYNTAX: &str = include_str!("../../cli/src/duka.sublime-syntax");

pub fn syntax_set() -> SyntaxSet {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    if let Ok(duka) = SyntaxDefinition::load_from_str(DUKA_SYNTAX, true, None) {
        builder.add(duka);
    }
    builder.build()
}

pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl Highlighter {
    pub fn new() -> Self {
        let syntaxes = syntax_set();
        let theme = ThemeSet::load_defaults()
            .themes
            .remove("base16-ocean.dark")
            .unwrap_or_default();
        Self { syntaxes, theme }
    }

    fn duka(&self) -> syntect::parsing::SyntaxReference {
        self.syntaxes
            .find_syntax_by_name("duka")
            .or_else(|| self.syntaxes.find_syntax_by_extension("duka"))
            .cloned()
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text().clone())
    }

    pub fn paint(&self, code: &str) -> String {
        let syntax = self.duka();
        let mut hl = HighlightLines::new(&syntax, &self.theme);
        let mut out = String::new();
        for line in LinesWithEndings::from(code) {
            let regions = match hl.highlight_line(line, &self.syntaxes) {
                Ok(regions) => regions,
                Err(_) => return code.to_owned(),
            };
            for (style, text) in regions {
                let _ = write!(out, "{}", escape(style, text));
            }
        }
        out.push_str("\x1b[0m");
        out
    }
}

fn escape(style: Style, text: &str) -> String {
    let foreground = style.foreground;
    format!(
        "\x1b[38;2;{};{};{}m{}",
        foreground.r, foreground.g, foreground.b, text
    )
}

pub struct SourceView {
    pub lines: Vec<String>,
}

impl SourceView {
    pub fn new(code: &str) -> Self {
        Self {
            lines: code.lines().map(str::to_owned).collect(),
        }
    }

    fn window(&self, around: usize, radius: usize) -> impl Iterator<Item = usize> {
        let start = around.saturating_sub(radius);
        let end = (around + radius).min(self.lines.len());
        (start..end).map(|i| i + 1)
    }

    pub fn show(
        &self,
        hl: &Highlighter,
        current: Option<usize>,
        from: Option<usize>,
        radius: usize,
    ) {
        let scope: Vec<usize> = match from {
            Some(at) => (at..=at.saturating_add(10)).collect(),
            None => match current {
                Some(at) => self.window(at, radius).collect(),
                None => (1..=self.lines.len().min(10)).collect(),
            },
        };
        for line in scope {
            let Some(text) = self.lines.get(line - 1) else {
                continue;
            };
            let marker = if Some(line) == current {
                "=> ".bright_green().bold().to_string()
            } else {
                "   ".to_string()
            };
            let dim = current.is_some_and(|c| c != line);
            let body = hl.paint(text);
            let num = format!("{line:>4} ").bright_blue();
            let body = if dim { body.dimmed().to_string() } else { body };
            println!("{marker}{num}{body}");
        }
    }
}

pub fn frame_name(frame: &DebugFrameInfo) -> String {
    frame
        .debug_name
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| "main".to_owned())
}

pub fn frame_line(frame: &DebugFrameInfo) -> String {
    match frame.pc_span {
        Some(span) => span.start.line.to_string(),
        None => "?".to_owned(),
    }
}

pub fn show_pause(reason: &str, frames: &[DebugFrameInfo], view: &SourceView, hl: &Highlighter) {
    let Some(top) = frames.first() else { return };
    let line = frame_line(top);
    println!(
        "{} {} in {} at line {} (depth {})",
        "Paused".bright_red().bold(),
        reason,
        frame_name(top).bright_white(),
        line.bright_white(),
        frames.len(),
    );
    if let Ok(line) = line.parse::<usize>() {
        view.show(hl, Some(line), None, 2);
    }
}

pub fn show_frames(frames: &[DebugFrameInfo], selected: usize) {
    for (i, frame) in frames.iter().enumerate() {
        let marker = if i == selected { "=> " } else { "   " };
        println!(
            "{marker}#{i} {} at {}:{}, pc {}",
            frame_name(frame),
            frame.source_name,
            frame_line(frame),
            frame.pc,
        );
    }
}

pub fn show_values(title: &str, values: &[(Box<str>, RuntimeValue)]) {
    if values.is_empty() {
        println!("{title}: (empty)");
        return;
    }
    let width = values.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, value) in values {
        println!("{title} {name:>width$} = {value}", title = title.green(),);
    }
}
