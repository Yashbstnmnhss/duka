use std::error::Error;
use std::fmt::Display;

use crate::parser::ast::{
    Block, DukaChunk, Expr, Field, FuncBody, If, Param, Path, PathSuffix, Stmt,
};
use crate::parser::ast::{ExprKind, StmtKind};
use duka_shared::types::{BinOp, DukaGenerator, UnOp};
use duka_shared::value::ConstValue;

#[derive(Debug)]
pub struct DebugTranspilerError {
    msg: String,
}
impl Display for DebugTranspilerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.msg)
    }
}
impl Error for DebugTranspilerError {}

pub struct DebugTranspiler {
    buffer: String,
    /// Space indent, tab for 4
    indent: usize,
}
impl DebugTranspiler {
    #[inline]
    fn increase(&mut self) {
        self.indent += 4;
    }
    #[inline]
    fn decrease(&mut self) {
        self.indent = self.indent.saturating_sub(4);
    }
    #[inline]
    fn newline(&mut self) {
        self.emit("\n");
    }
    #[inline]
    fn emit(&mut self, content: &str) {
        self.buffer.push_str(content);
    }
    #[inline]
    fn emit_ident(&mut self, content: &str) {
        self.buffer.push_str(&" ".repeat(self.indent));
        self.buffer.push_str(content);
    }
    #[inline]
    fn emit_newline(&mut self, content: &str) {
        self.emit_ident(content);
        self.newline();
    }

    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            indent: 0,
        }
    }

    fn gen_list<T>(
        &mut self,
        ts: Box<[T]>,
        f: fn(&mut Self, T) -> Result<(), String>,
        newline: bool,
    ) -> Result<(), String> {
        let len = ts.len();
        for (i, t) in ts.into_iter().enumerate() {
            if newline {
                self.emit_ident("");
            }
            f(self, t)?;
            if i != len - 1 {
                self.emit(", ");
            }
            if newline {
                self.newline();
            }
        }
        Ok(())
    }

    pub fn gen_func_body(
        &mut self,
        FuncBody(params, _, _, blk): FuncBody,
        newline: bool,
    ) -> Result<(), String> {
        self.emit("(");
        self.gen_list(
            params,
            |st, p| {
                match p {
                    Param::Name(n) | Param::Typed(n, _) => st.emit(&n.0),
                    Param::Var(_) => st.emit("..."),
                }
                Ok(())
            },
            false,
        )?;
        self.emit(")");
        self.newline();
        self.gen_block(*blk)?;
        self.emit_ident("end");
        if newline {
            self.newline();
        }
        Ok(())
    }

    pub fn gen_path(&mut self, mut path: Path) -> Result<(), String> {
        let mut collected = vec![];
        loop {
            match path {
                Path::Base(b) => {
                    self.emit(&b.0);
                    break;
                }
                Path::Expr(e) => {
                    let simple = Self::is_simple_expr(&*e);
                    if !simple {
                        self.emit("(");
                    }
                    self.gen_expr(*e)?;
                    if !simple {
                        self.emit(")");
                    }
                    break;
                }
                Path::Chain(parent, chain) => {
                    collected.push(chain);
                    path = *parent;
                }
            }
        }
        for c in collected {
            match c {
                PathSuffix::Colon(c) => {
                    self.emit(":");
                    self.emit(&c.0);
                }
                PathSuffix::Dot(d) => {
                    self.emit(".");
                    self.emit(&d.0);
                }
                PathSuffix::Index(i) => {
                    self.emit("[");
                    self.gen_expr(*i)?;
                    self.emit("]");
                }
                PathSuffix::TypeArgs(_, _) => {
                    self.emit(".<TYPES>");
                }
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn is_simple_expr(expr: &Expr) -> bool {
        matches!(expr, Expr(ek, _) if matches!(ek, ExprKind::Access(..) | ExprKind::Call(..) | ExprKind::Literal(..)))
    }

    pub fn gen_expr(&mut self, Expr(kind, _): Expr) -> Result<(), String> {
        match kind {
            ExprKind::Empty => self.emit("[=[EMPTY]=]"),
            ExprKind::VarArg => self.emit("..."),
            ExprKind::Literal(val) => match val {
                ConstValue::Bool(b) => self.emit(&b.to_string()),
                ConstValue::Nil => self.emit("nil"),
                ConstValue::Int(i) => self.emit(&i.to_string()),
                ConstValue::Float(f) => self.emit(&f.to_string()),
                ConstValue::String(bytes) => {
                    self.emit("\"");
                    self.emit(str::from_utf8(&bytes).map_err(|e| e.to_string())?);
                    self.emit("\"");
                }
            },
            ExprKind::Do(blk) => {
                self.emit("do");
                self.newline();
                self.gen_block(*blk)?;
                self.emit_newline("end");
            }
            ExprKind::Access(path) => {
                self.gen_path(*path)?;
            }
            ExprKind::Call(expr, exprs) => {
                let simple = Self::is_simple_expr(&*expr);
                if !simple {
                    self.emit("(");
                }
                self.gen_expr(*expr)?;
                if !simple {
                    self.emit(")");
                }
                self.emit("(");
                self.gen_list(exprs, Self::gen_expr, false)?;
                self.emit(")");
            }
            ExprKind::SysCall(sys_call) => {
                self.emit(&format!("[=[{sys_call:?}]=]"));
            }
            ExprKind::Table(fields) => {
                self.emit("{");
                self.newline();
                self.increase();
                self.gen_list(
                    fields,
                    |ts, field| {
                        match field {
                            Field::KeyValue(k, v) => {
                                ts.emit_ident("[");
                                ts.gen_expr(k)?;
                                ts.emit("] = ");
                                ts.gen_expr(v)?;
                            }
                            Field::NameValue(n, v) => {
                                ts.emit_ident(&n.0);
                                ts.emit(" = ");
                                ts.gen_expr(v)?;
                            }
                            Field::Value(e) => {
                                ts.emit_ident("");
                                ts.gen_expr(e)?;
                            }
                        }
                        Ok(())
                    },
                    true,
                )?;
                self.decrease();
                self.newline();
                self.emit_ident("}");
            }
            ExprKind::Array(exprs) => {
                self.emit("[");
                self.gen_list(exprs, Self::gen_expr, false)?;
                self.emit_ident("]");
            }
            ExprKind::Function(func_body) => {
                self.emit("function");
                self.gen_func_body(func_body, false)?;
            }
            ExprKind::Unary(expr, un_op) => {
                self.emit(match un_op {
                    UnOp::Length => "#",
                    UnOp::Not => "not ",
                    UnOp::BitNot => "~",
                    UnOp::Minus => "-",
                });
                self.gen_expr(*expr)?;
            }
            ExprKind::Binary(a, b, bin_op) => {
                self.gen_expr(*a)?;
                self.emit(" ");
                self.emit(match bin_op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Multiply => "*",
                    BinOp::Divide => "/",
                    BinOp::IDivide => "//",
                    BinOp::Mod => "%",
                    BinOp::Pow => "**",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    BinOp::Xor => "xor",
                    BinOp::Equal => "==",
                    BinOp::NotEqual => "!=",
                    BinOp::Greater => ">",
                    BinOp::Less => "<",
                    BinOp::GreaterEqual => ">=",
                    BinOp::LessEqual => "<=",
                    BinOp::BitAnd => "&",
                    BinOp::BitOr => "|",
                    BinOp::BitXor => "^",
                    BinOp::ShiftL => "<<",
                    BinOp::ShiftR => ">>",
                    BinOp::Concat => "..",
                    BinOp::Pipeline(_) => "|>",
                    BinOp::PipelineL => "<|",
                });
                self.emit(" ");
                self.gen_expr(*b)?;
            }
            ExprKind::If(bi) => self.gen_if(*bi)?,
            ExprKind::TypeLit(td) => self.emit(&td.to_string()),
            ek if ek.is_sugar() => return Err(format!("{ek} is unsupported")),
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn gen_if(&mut self, If(if_, elseifs, else_): If) -> Result<(), String> {
        self.emit("if ");
        self.gen_expr(*if_.1)?;
        self.emit(" then");
        self.newline();
        self.gen_block(*if_.0)?;
        self.newline();

        for elseif in elseifs {
            self.emit_ident("elseif ");
            self.gen_expr(*elseif.1)?;
            self.emit(" then");
            self.newline();
            self.gen_block(*elseif.0)?;
            self.newline();
        }

        if let Some(else_) = else_ {
            self.emit_ident("else");
            self.newline();
            self.gen_block(*else_)?;
            self.newline();
        }

        self.emit_ident("end");

        Ok(())
    }

    pub fn gen_stmt(&mut self, Stmt(kind, _): Stmt) -> Result<(), String> {
        match kind {
            StmtKind::Empty => self.emit_newline(";"),
            StmtKind::Expr(expr) => {
                self.emit_ident("(");
                self.gen_expr(*expr)?;
                self.emit(")");
                self.newline();
            }
            StmtKind::Call(expr, exprs) => {
                self.emit_ident("");
                self.gen_expr(*expr)?;
                self.emit("(");
                self.gen_list(exprs, Self::gen_expr, false)?;
                self.emit(")");
                self.newline();
            }
            StmtKind::Label(name) => self.emit_newline(&format!("::{name}::")),
            StmtKind::Goto(to) => self.emit_newline(&format!("goto {to}")),
            StmtKind::Break => self.emit_newline("break"),
            StmtKind::Continue => self.emit_newline("continue"),
            StmtKind::Return(exprs, bang) => {
                self.emit_ident("return");
                if bang {
                    self.emit("!");
                }
                self.emit(" ");
                self.gen_list(exprs, Self::gen_expr, false)?;
                self.newline();
            }
            StmtKind::If(bi) => {
                self.emit_ident("");
                self.gen_if(bi)?;
                self.newline();
            }
            StmtKind::ForNumeric(path, expr, expr1, expr2, blk) => {
                self.emit_ident("for ");
                self.gen_path(path)?;
                self.emit(" = ");
                let mut exprs = vec![*expr, *expr1];
                if let Some(expr2) = expr2 {
                    exprs.push(*expr2);
                }
                self.gen_list(exprs.into_boxed_slice(), Self::gen_expr, false)?;
                self.emit(" do");
                self.newline();
                self.gen_block(*blk)?;
                self.newline();
                self.emit_newline("end");
            }
            StmtKind::ForGeneric(paths, exprs, blk, bang) => {
                self.emit_ident("for");
                if bang {
                    self.emit("!");
                }
                self.emit(" ");
                self.gen_list(paths, Self::gen_path, false)?;
                self.emit(" in ");
                self.gen_list(exprs, Self::gen_expr, false)?;
                self.emit(" do");
                self.newline();
                self.gen_block(*blk)?;
                self.newline();
                self.emit_newline("end");
            }
            StmtKind::While(expr, blk, bang) => {
                self.emit_ident("while");
                if bang {
                    self.emit("!");
                }
                self.emit(" ");
                self.gen_expr(*expr)?;
                self.emit(" do");
                self.newline();
                self.gen_block(*blk)?;
                self.newline();
                self.emit_newline("end");
            }
            StmtKind::Do(blk, bang) => {
                self.emit_ident("do");
                if bang {
                    self.emit("!");
                }
                self.newline();
                self.gen_block(*blk)?;
                self.emit_newline("end");
            }
            StmtKind::Assign(paths, exprs) => {
                self.emit_ident("");
                self.gen_list(paths, Self::gen_path, false)?;
                self.emit(" = ");
                self.gen_list(exprs, Self::gen_expr, false)?;
                self.newline();
            }
            StmtKind::Define(items, exprs, global, bang) => {
                self.emit_ident(if global { "global" } else { "local" });
                if bang {
                    self.emit("!");
                }
                self.emit(" ");
                self.gen_list(
                    items,
                    |ts, item| {
                        ts.emit(&item.0.0.0);
                        Ok(())
                    },
                    false,
                )?;
                self.emit(" = ");
                self.gen_list(exprs, Self::gen_expr, false)?;
                self.newline();
            }
            StmtKind::Function(path, _, func_body, _) => {
                self.emit_ident("function ");
                self.gen_path(path)?;
                self.gen_func_body(*func_body, true)?;
                self.newline();
            }
            StmtKind::TypeAlias(name, td) => {
                self.emit_ident("type ");
                self.emit(&name.0);
                self.emit(" = ");
                self.emit(&td.to_string());
                self.newline();
            }
            StmtKind::TypeFunction(name, func_body) => {
                self.emit_ident("type function ");
                self.emit(&name.0);
                self.gen_func_body(*func_body, true)?;
                self.newline();
            }
            StmtKind::InlineTypeFunction(name, params, td) => {
                self.emit_ident("type function ");
                self.emit(&name.0);
                self.emit("(");
                self.gen_list(
                    params,
                    |st, p| {
                        match p {
                            Param::Name(n) | Param::Typed(n, _) => st.emit(&n.0),
                            Param::Var(_) => st.emit("..."),
                        }
                        Ok(())
                    },
                    false,
                )?;
                self.emit(") = ");
                self.emit(&td.to_string());
                self.newline();
            }
            sk if sk.is_sugar() => return Err(format!("{sk} is unsupported")),
            _ => unreachable!(),
        }
        Ok(())
    }

    pub fn gen_block(&mut self, blk: Block) -> Result<(), String> {
        self.increase();
        for stmt in blk.0 {
            self.gen_stmt(stmt)?;
        }
        if let Some(ret) = blk.1 {
            self.gen_stmt(*ret)?;
        }
        self.decrease();
        Ok(())
    }

    pub fn gen_main(&mut self, blk: Block) -> Result<(), String> {
        for stmt in blk.0 {
            self.gen_stmt(stmt)?;
        }
        if let Some(ret) = blk.1 {
            self.gen_stmt(*ret)?;
        }
        Ok(())
    }
}

impl DukaGenerator<String, DebugTranspilerError> for DebugTranspiler {
    type ConfigType = ();
    type InputType = DukaChunk;

    fn generate(
        input: Self::InputType,
        _config: Self::ConfigType,
    ) -> Result<String, DebugTranspilerError> {
        let mut generator = Self::new();
        generator
            .gen_main(input.block)
            .map_err(|msg| DebugTranspilerError { msg })?;
        Ok(generator.buffer)
    }
}
