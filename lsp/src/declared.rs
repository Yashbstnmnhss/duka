//! The standard library, written out as declarations.

use std::collections::HashMap;
use std::fmt::Write as _;

use duka_lib::duka_shared::docs::{MetaInfo, MetaItemInfo, ParamMeta, ReturnMeta};
use tower_lsp::lsp_types::Url;

use crate::docs;

/// The scheme the editor resolves these files under. A path that ends in `.duka`
/// so the editor picks the DUKA grammar for it without being told.
pub const SCHEME: &str = "duka-virtual";

/// What every generated file opens with. A blank line follows it, so this is the
/// file talking about itself rather than documenting its first declaration.
const PREAMBLE: &str = "\
--- The Builtins
";

/// One generated file, and where inside it each builtin landed.
#[derive(Clone)]
pub struct Generated {
    /// The file name, such as `os.duka`.
    pub name: String,
    pub text: String,
    /// Written as a path: `print`, `os.exit`, `Pick`.
    pub line_of: HashMap<String, u32>,
}

impl Generated {
    pub fn uri(&self) -> Url {
        Url::parse(&format!("{SCHEME}:/{name}", name = self.name))
            .expect("a generated file name is a valid path")
    }

    pub fn line_of(&self, path: &str) -> Option<u32> {
        self.line_of.get(path).copied()
    }
}

/// A builder that keeps track of the line it is writing, because a declaration
/// file is only useful if the editor can be told *where* in it to go.
struct Writer {
    text: String,
    line: u32,
    line_of: HashMap<String, u32>,
}

impl Writer {
    fn new(header: &str) -> Self {
        let mut w = Writer {
            text: String::new(),
            line: 0,
            line_of: HashMap::new(),
        };
        w.raw(header);
        w
    }

    fn raw(&mut self, line: &str) {
        for l in line.split('\n') {
            self.text.push_str(l);
            self.text.push('\n');
            self.line += 1;
        }
    }

    // fn blank(&mut self) {
    //     if !self.text.ends_with("\n\n") {
    //         self.raw("");
    //     }
    // }

    /// A comment run, written the way a project writes documentation. Every line
    /// of prose is prefixed, and an empty one is written bare so the run stays a
    /// run: a comment separated from what it documents does not attach to it.
    fn doc(&mut self, text: &str) {
        let body = text.trim();
        if body.is_empty() {
            return;
        }
        if !self.text.ends_with("\n\n") && !self.text.is_empty() {
            self.text.push('\n');
            self.line += 1;
        }
        for l in body.lines() {
            let l = l.trim();
            if l.is_empty() {
                self.raw("---");
            } else {
                self.raw(&format!("--- {l}"));
            }
        }
    }

    /// Writes one declaration and remembers the line it is on.
    fn declare(&mut self, path: &str, meta: &MetaInfo) {
        let Some(decl) = declaration(meta) else {
            return;
        };
        if !self.text.ends_with("\n\n") && !self.text.is_empty() {
            self.text.push('\n');
            self.line += 1;
        }
        self.doc(meta.doc);
        self.line_of.insert(path.to_owned(), self.line);
        self.raw(&decl);
        self.raw("");
    }
}

/// The declaration for one builtin, written the way a project writes it.
///
/// Returns `None` for a module: a module is a table of what is in it rather than
/// something declared, so it is a section here rather than a line.
fn declaration(meta: &MetaInfo) -> Option<String> {
    Some(match &meta.info {
        MetaItemInfo::Function { returns, params } => {
            let params = written_params(params);
            format!(
                "@declare function {}{}",
                writable(meta.name)?,
                signature_tail(&params, returns)
            )
        }
        MetaItemInfo::Static { inner } => return declaration(inner),
        MetaItemInfo::Constant { ty, .. } => {
            // a declaration takes no value: there would be nothing to do with one
            format!("@declare @const local {}: {ty}", writable(meta.name)?)
        }
        // `@declare` is available for a function, a value and an object -- not for
        // a type function, which is a type rather than something the runtime
        // provides. There is still somewhere for one to land, so it is written out
        // as a comment: landing on a line that says exactly what the thing is
        // beats refusing to move.
        MetaItemInfo::TypeFunction { param_count } => {
            let params = vec!["any"; *param_count].join(", ");
            format!(
                "-- type function {}({params}): {}",
                writable(meta.name)?,
                meta.get_type()
            )
        }
        MetaItemInfo::UserData { ty_name, methods } => {
            let methods = methods
                .iter()
                .map(|m| m.name)
                .collect::<Vec<_>>()
                .join(", ");
            format!("@declare userdata {ty_name}({methods})")
        }
        MetaItemInfo::Module { .. } => return None,
    })
}

/// A name the language will not accept where a declaration wants one.
///
/// A builtin may be registered under a keyword -- `io.type` is one -- and there
/// is no way to write it. A declaration that does not parse is worse than no
/// declaration, because it puts an error in a file the reader opened on purpose,
/// so such a name is left out and the prose above it still stands.
fn writable(name: &str) -> Option<&str> {
    (!is_reserved(name)).then_some(name)
}

fn written_params(params: &'static [ParamMeta]) -> Vec<ParamMeta> {
    params
        .iter()
        .filter(|p| !is_reserved(p.name))
        .cloned()
        .collect()
}

const RESERVED: &[&str] = &[
    "end", "if", "else", "for", "while", "return", "local", "function", "type", "object", "global",
    "export", "const", "do", "break", "continue", "then", "self",
];

#[inline]
fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

/// `(s: string, ...): string` -- what a declaration needs in order to be one: the
/// parameters with their types, and what it hands back.
///
/// A default is left out. The registration writes what a human would read
/// ("Assertion failed", `- 1`, `"r".to_owned()`), and those are not expressions
/// the language accepts. What a parameter falls back to belongs in the prose
/// above it, which is where a reader is looking for it anyway.
///
/// `ReturnMeta` renders as `[count] text` because that is what a hover wants to
/// say. A declaration wants the type, so it is built from `tys` instead: nothing
/// written for no return, one type written bare, several as a tuple.
fn signature_tail(params: &[ParamMeta], returns: &ReturnMeta) -> String {
    let mut out = String::from("(");
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if p.var_arg {
            out.push_str("...");
            continue;
        }
        let ty = p.ty.to_string();
        let ty = parenthesise(&ty);
        // an optional parameter is `ty | nil`, not a `?` after the name: the `?`
        // in `name?` is how a hover reads it, not how the language writes it
        let ty = if p.optional {
            format!("{ty} | nil")
        } else {
            ty
        };
        let _ = write!(out, "{}: {}", p.name, ty);
    }
    out.push(')');
    let tys: Vec<String> = returns.tys.iter().map(|t| t.to_string()).collect();
    match tys.len() {
        0 => {}
        1 => {
            let _ = write!(out, ": {}", tys[0]);
        }
        _ => {
            let _ = write!(out, ": [{}]", tys.join(", "));
        }
    }
    if returns.var_arg {
        out.push_str(if tys.is_empty() { ": ..." } else { ", ..." });
    }
    out
}

/// A union needs parentheses where a parameter is read: `(function | table)`
/// parses, and `function | table` reads as the start of another declaration.
fn parenthesise(ty: &str) -> String {
    if ty.contains(" | ") || ty.contains(" & ") {
        format!("({ty})")
    } else {
        ty.to_owned()
    }
}

fn core_members() -> &'static [MetaInfo] {
    docs::value_builtin_metas()
        .iter()
        .find(|m| m.name == "core")
        .and_then(docs::as_module)
        .unwrap_or_default()
}

/// Every file worth generating: the global builtins, one per module, and the type
/// context builtins.
pub fn generated() -> Vec<Generated> {
    let mut out = vec![];
    let metas = docs::value_builtin_metas();

    let mut core = Writer::new(PREAMBLE);
    for member in core_members() {
        core.declare(member.name, member);
    }
    out.push(Generated {
        name: "core.duka".to_owned(),
        text: core.text,
        line_of: core.line_of,
    });

    for module in metas.iter().filter(|m| m.name != "core") {
        let Some(inner) = docs::as_module(module) else {
            continue;
        };
        let header = format!(
            "{PREAMBLE}\n\
             --- This file is `{name}` (aka the module name)\n",
            name = module.name
        );
        let mut w = Writer::new(&header);
        for member in inner {
            w.declare(&format!("{}.{}", module.name, member.name), member);
        }
        out.push(Generated {
            name: format!("{}.duka", module.name),
            text: w.text,
            line_of: w.line_of,
        });
    }

    let type_header = format!(
        "{PREAMBLE}\n\
         --- This file is the type context builtins.\n"
    );
    let mut types = Writer::new(&type_header);
    for meta in docs::type_builtin_metas() {
        types.declare(meta.name, meta);
    }
    out.push(Generated {
        name: "type.duka".to_owned(),
        text: types.text,
        line_of: types.line_of,
    });

    out
}

/// The generated file a builtin is written in, and the line it is on.
///
/// `path` is what the reader wrote: `print`, `os.exit`, `Pick`.
pub fn locate(path: &str) -> Option<(Url, u32)> {
    generated()
        .into_iter()
        .find_map(|file| Some((file.uri(), file.line_of(path)?)))
}
#[cfg(test)]
mod tests {
    use super::*;
    use duka_lib::duka_frontend::lexer::LexerWithMacro;
    use duka_lib::duka_frontend::parser::Parser;
    use duka_lib::duka_shared::config::DukaLexerConfig;
    use duka_lib::duka_shared::types::{DukaLexer, SourceName};
    use std::io::Cursor;

    fn parse(text: &str) -> Vec<String> {
        let lexer = LexerWithMacro::new(
            Cursor::new(text),
            SourceName::Virtual("declared".into()),
            DukaLexerConfig { keep_comment: true },
        );
        let tokens = lexer.tokenize().expect("lexes");
        let (_chunk, errs) = Parser::parse_lenient(tokens, Default::default());
        errs.iter().map(|e| e.to_string()).collect()
    }

    /// A generated file that does not parse puts an error in a file the reader
    /// opened on purpose, so this is the test that matters most here: the text is
    /// only worth generating if the language will read it.
    #[test]
    fn every_generated_file_is_a_file_the_language_accepts() {
        let mut bad = vec![];
        for file in generated() {
            for err in parse(&file.text) {
                bad.push(format!("{}: {err}", file.name));
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn a_module_is_a_file_and_a_builtin_is_a_line_in_it() {
        let files = generated();
        assert!(
            files.iter().any(|f| f.name == "os.duka"),
            "one file per module"
        );
        let os = files.iter().find(|f| f.name == "os.duka").unwrap();
        assert!(os.text.contains("@declare function exit("), "{}", os.text);
        let (uri, line) = locate("os.exit").expect("os.exit is somewhere to land");
        assert_eq!(uri.as_str(), "duka-virtual:/os.duka");
        assert_eq!(
            os.text.lines().nth(line as usize).unwrap(),
            "@declare function exit(code: int | nil)"
        );
    }

    /// A global builtin is a member of core, so `print` belongs in the file about
    /// the globals rather than in one named after a module it is not in.
    #[test]
    fn a_global_builtin_lands_in_core() {
        let (uri, _) = locate("print").expect("print is somewhere to land");
        assert_eq!(uri.as_str(), "duka-virtual:/core.duka");
    }

    /// A default in the registration is written for a reader ("Assertion failed",
    /// `"r".to_owned()`), not as an expression, so it cannot become one. The
    /// parameter keeps its place and its type.
    #[test]
    fn a_parameter_default_is_left_to_the_prose() {
        let os = generated()
            .iter()
            .find(|f| f.name == "os.duka")
            .cloned()
            .unwrap();
        assert!(
            os.text.contains("@declare function exit(code: int | nil)"),
            "{}",
            os.text
        );
        assert!(
            !os.text.contains("exit(code: int | nil = "),
            "a default is not an expression: {}",
            os.text
        );
    }

    /// `@declare` says what exists rather than what to build, and a builtin is
    /// exactly that. Every value builtin is one, so every file is declarations.
    #[test]
    fn everything_in_a_generated_file_is_a_declaration() {
        for file in generated() {
            for line in file
                .text
                .lines()
                .filter(|l| !l.trim_start().starts_with("--"))
            {
                assert!(
                    line.trim().is_empty() || line.starts_with("@declare"),
                    "{}: {line}",
                    file.name
                );
            }
        }
    }
}
