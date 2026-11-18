use colored::Colorize;
use duka_lib::duka_shared::types::{SourceName, ValueCount};
use duka_lib::value::RuntimeValue;
use duka_lib::vm::VM;
use duka_lib::vm::coroutine::DebuggerCell;
use duka_lib::vm::debugger::{DebugAction, DebugFrameInfo};
use rustyline::{Editor, error::ReadlineError, history::FileHistory};

use crate::helper::DbgHelper;
use crate::render::{self, Highlighter, SourceView};

pub enum Flow {
    Resume,
    Quit,
}

pub struct Session {
    vm: Option<VM>,
    debugger: DebuggerCell,
    entry: SourceName,
    view: SourceView,
    hl: Highlighter,
    selected: usize,
    pending: Option<&'static str>,
    last: Option<String>,
}

impl Session {
    pub fn new(vm: VM, debugger: DebuggerCell, entry: SourceName, code: String) -> Self {
        Self {
            vm: Some(vm),
            debugger,
            entry,
            view: SourceView::new(&code),
            hl: Highlighter::new(),
            selected: 0,
            pending: None,
            last: None,
        }
    }

    pub fn vm_mut(&mut self) -> &mut VM {
        self.vm.as_mut().unwrap()
    }

    pub fn add_breakpoint(&mut self, line: u32) {
        self.debugger
            .borrow_mut()
            .add_breakpoint(line, self.entry.clone());
        println!("{}", format!("Breakpoint set at line {line}").bright_cyan());
    }

    pub fn reason(&self) -> &'static str {
        if self.debugger.borrow().paused_on_error() {
            return "on runtime error";
        }
        let line = self
            .frames()
            .first()
            .and_then(|f| f.pc_span)
            .map(|s| (s.start.line, s.end.line));
        let hit = self.debugger.borrow().get_breakpoints().iter().any(|bp| {
            bp.enabled && line.is_some_and(|(start, end)| bp.line == start || bp.line == end)
        });
        if hit {
            "on breakpoint"
        } else {
            self.pending.unwrap_or("after step")
        }
    }

    pub fn finish(&mut self, count: ValueCount) -> Result<Vec<RuntimeValue>, String> {
        let vm = self.vm.take().unwrap();
        let values = {
            let mut main = vm.main_coroutine_mut();
            let mut state = std::mem::take(&mut main.inner);
            state.take_stack_many(0, count).map_err(|e| e.to_string())?
        };
        std::mem::forget(vm);
        Ok(values.into())
    }

    pub fn on_pause(&mut self, reason: &str, rl: &mut Editor<DbgHelper, FileHistory>) -> Flow {
        let frames = self.frames();
        self.selected = 0;
        render::show_pause(reason, &frames, &self.view, &self.hl);
        loop {
            let line = match rl.readline("(duka-dbg) ") {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => {
                    println!("{}", "(continue to resume, quit to exit)".dimmed());
                    continue;
                }
                Err(ReadlineError::Eof) => return Flow::Quit,
                Err(e) => {
                    println!("{}", format!("readline: {e}").red());
                    return Flow::Quit;
                }
            };
            let _ = rl.add_history_entry(line.trim());
            match self.dispatch(line.trim()) {
                Ok(Some(flow)) => return flow,
                Ok(None) => {}
                Err(msg) => println!("{}", msg.red()),
            }
        }
    }

    fn frames(&self) -> Vec<DebugFrameInfo> {
        let mut frames = self.debugger.borrow().get_frames_info().to_vec();
        frames.reverse();
        for (i, frame) in frames.iter_mut().enumerate() {
            frame.depth = i;
        }
        frames
    }

    fn dispatch(&mut self, line: &str) -> Result<Option<Flow>, String> {
        let mut parts = line.split_whitespace();
        let Some(head) = parts.next() else {
            return self.repeat();
        };
        let arg = parts.next();
        match head {
            "b" | "break" => self.break_cmd(arg),
            "d" | "delete" | "del" => self.delete_cmd(arg),
            "breaks" | "info" => self.breaks_cmd(),
            "c" | "continue" => self.resume_cmd(DebugAction::Continue, line),
            "n" | "next" => self.resume_cmd(DebugAction::StepOver, line),
            "s" | "step" => self.resume_cmd(DebugAction::StepIn, line),
            "out" | "finish" => self.resume_cmd(DebugAction::StepOut, line),
            "bt" | "backtrace" | "frames" => self.frames_cmd(),
            "f" | "frame" => self.frame_cmd(arg),
            "params" | "args" => self.values_cmd(true),
            "locals" | "loc" => self.values_cmd(false),
            "l" | "list" => self.list_cmd(arg),
            "q" | "quit" => Ok(Some(Flow::Quit)),
            "h" | "help" => {
                self.help();
                Ok(None)
            }
            other => Err(format!("unknown command: {other}, try help")),
        }
    }

    fn repeat(&mut self) -> Result<Option<Flow>, String> {
        match self.last.take() {
            Some(cmd) => self.dispatch(&cmd),
            None => Err("empty command, try help".to_owned()),
        }
    }

    fn break_cmd(&mut self, arg: Option<&str>) -> Result<Option<Flow>, String> {
        let line = parse_line(arg)?;
        self.add_breakpoint(line);
        Ok(None)
    }

    fn delete_cmd(&mut self, arg: Option<&str>) -> Result<Option<Flow>, String> {
        let line = parse_line(arg)?;
        self.debugger.borrow_mut().remove_breakpoint(line);
        println!(
            "{}",
            format!("Breakpoint at line {line} removed").bright_cyan()
        );
        Ok(None)
    }

    fn breaks_cmd(&self) -> Result<Option<Flow>, String> {
        let debugger = self.debugger.borrow();
        let bps = debugger.get_breakpoints();
        if bps.is_empty() {
            println!("no breakpoints");
            return Ok(None);
        }
        for bp in bps {
            let state = if bp.enabled { "on" } else { "off" };
            println!("breakpoint at {}:{} ({state})", bp.source_name, bp.line);
        }
        Ok(None)
    }

    fn resume_cmd(&mut self, action: DebugAction, raw: &str) -> Result<Option<Flow>, String> {
        let on_error = self.debugger.borrow().paused_on_error();
        if on_error {
            self.debugger.borrow_mut().break_on_error(false);
        }
        let depth = self.frames().len();
        self.debugger.borrow_mut().set_action(action, depth);
        self.pending = Some(match action {
            DebugAction::Continue => "on breakpoint",
            _ => "after step",
        });
        self.last = Some(raw.to_owned());
        Ok(Some(Flow::Resume))
    }

    fn frames_cmd(&self) -> Result<Option<Flow>, String> {
        render::show_frames(&self.frames(), self.selected);
        Ok(None)
    }

    fn frame_cmd(&mut self, arg: Option<&str>) -> Result<Option<Flow>, String> {
        let frames = self.frames();
        let at = arg
            .map(|n| {
                n.parse::<usize>()
                    .map_err(|_| "frame index must be a number".to_owned())
            })
            .transpose()?;
        let at = at.unwrap_or(0);
        if at >= frames.len() {
            return Err(format!(
                "no frame {at}, {} frame(s) on the stack",
                frames.len()
            ));
        }
        self.selected = at;
        let frame = &frames[at];
        println!(
            "{} {} at {}:{}",
            format!("#{at}").bright_cyan(),
            render::frame_name(frame),
            frame.source_name,
            render::frame_line(frame),
        );
        Ok(None)
    }

    fn values_cmd(&self, params: bool) -> Result<Option<Flow>, String> {
        let frames = self.frames();
        let Some(frame) = frames.get(self.selected) else {
            return Err("not paused on a frame".to_owned());
        };
        let vm = self.vm.as_ref().unwrap();
        let values = if params {
            vm.scheduler.inspect_params(frame)
        } else {
            vm.scheduler.inspect_locals(frame)
        };
        render::show_values(if params { "param" } else { "local" }, &values);
        Ok(None)
    }

    fn list_cmd(&mut self, arg: Option<&str>) -> Result<Option<Flow>, String> {
        let current = self
            .frames()
            .first()
            .and_then(|f| f.pc_span)
            .map(|s| s.start.line as usize);
        let from = arg
            .map(|n| n.parse::<usize>())
            .transpose()
            .map_err(|_| "list expects a line number".to_owned())?;
        self.view.show(&self.hl, current, from, 4);
        Ok(None)
    }

    fn help(&self) {
        println!("{}", "duka-dbg commands".bright_blue());
        for line in [
            "b[reak] <line>      set a breakpoint",
            "d[elete] <line>     remove a breakpoint",
            "breaks              list breakpoints",
            "c[ontinue]          run to the next breakpoint",
            "n[ext]              step over a line",
            "s[tep]              step into a line",
            "out | finish        step out of the current frame",
            "bt | frames         show the call stack",
            "f[rame] [n]         select a frame (innermost by default)",
            "params | args       show parameters of the selected frame",
            "locals | loc        show locals of the selected frame",
            "l[ist] [line]       show source around the paused line",
            "q[uit]              leave the debugger",
            "h[elp]              this list",
            "<enter>             repeat the last stepping command",
        ] {
            println!("  {line}");
        }
    }
}

fn parse_line(arg: Option<&str>) -> Result<u32, String> {
    arg.map(|n| n.parse::<u32>())
        .transpose()
        .map_err(|_| "a line number is required".to_owned())?
        .filter(|l| *l > 0)
        .ok_or_else(|| "a line number is required".to_owned())
}
