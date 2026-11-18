use duka_shared::{errors::Span, types::SourceName};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebugAction {
    #[default]
    Continue,
    StepIn,
    StepOver,
    StepOut,
}

#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub line: u32,
    pub enabled: bool,
    pub source_name: SourceName,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DebugFrameInfo {
    pub debug_name: Option<Box<str>>,
    pub source_name: SourceName,
    /// 此pc的指令对应的代码span
    pub pc_span: Option<Span>,
    pub pc: usize,
    pub base: usize,
    pub stack_len: usize,

    pub narg: usize,
    pub depth: usize,
    pub param_count: usize,
    pub has_var_arg: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Debugger {
    pub(crate) breakpoints: Vec<Breakpoint>,
    pub(crate) mode: Option<DebugAction>,
    pub(crate) pause_depth: Option<usize>,
    pub(crate) paused_frames: Vec<DebugFrameInfo>,
    pub(crate) break_on_error: bool,
    pub(crate) paused_on_error: bool,
    pub(crate) last_pause_span: Option<Span>,
    pub(crate) bp_skip: Option<(std::ops::Range<usize>, Span)>,
}

impl Debugger {
    pub fn paused_on_error(&self) -> bool {
        self.paused_on_error
    }

    pub fn get_breakpoints(&self) -> &[Breakpoint] {
        &self.breakpoints
    }

    pub fn break_on_error(&mut self, val: bool) {
        self.break_on_error = val
    }

    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
    }
    pub fn add_breakpoint(&mut self, line: u32, source: SourceName) {
        if !self.breakpoints.iter().any(|b| b.line == line) {
            self.breakpoints.push(Breakpoint {
                line,
                enabled: true,
                source_name: source,
            });
        }
    }
    pub fn remove_breakpoint(&mut self, line: u32) {
        self.breakpoints.retain(|b| b.line != line)
    }

    pub fn set_action(&mut self, action: DebugAction, depth: usize) {
        self.mode = Some(action);
        self.pause_depth = Some(depth);
        self.paused_on_error = false;
    }

    pub fn get_frames_info(&self) -> &[DebugFrameInfo] {
        &self.paused_frames
    }
}
