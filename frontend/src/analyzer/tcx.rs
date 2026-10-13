//! The type level: what may be written where a type is expected.
//!

use duka_shared::{errors::Span, types::UnOp};

use crate::{
    analyzer::{Visit, Visitor},
    parser::ast::{
        AttrName, Block, Expr, Field, If, Match, NodeKind, Path, PathSuffix, Pattern, PatternTerm,
        Stmt,
    },
};

/// Walks one expression and remembers the first node that is not part of the
/// type-level subset. The traversal comes from the derived visitor, so a node
/// added to the language later is covered without touching this.
pub struct TypeLevelGate {
    offender: Option<Span>,
}

impl TypeLevelGate {
    /// The span of the first construct in `expr` the type level cannot
    /// evaluate, if there is one.
    pub fn first_offender(expr: &Expr) -> Option<Span> {
        let mut gate = TypeLevelGate { offender: None };
        expr.visit(&mut gate);
        gate.offender
    }
}

impl Visitor for TypeLevelGate {
    fn visit_node(&mut self, stmt: &Stmt) {
        if self.offender.is_none() && !stmt.0.is_tcx() {
            self.offender = Some(stmt.1);
        }
    }
}

/// Renders a type-level expression back to something readable. This covers the
/// subset above, which is all that can reach a type position; anything else is
/// printed as its shape rather than guessed at. It is the display half of the
/// same question the gate answers on the checking half.
pub fn render(expr: &Expr) -> String {
    match &expr.0 {
        NodeKind::Empty => "never".into(),
        NodeKind::VarArg => "...".into(),
        NodeKind::Literal(cv) => cv.to_string(),
        NodeKind::TypeLit(td) => td.to_string(),
        NodeKind::Access(path) => render_path(path),
        NodeKind::Call(callee, args) => format!(
            "{}({})",
            render(callee),
            args.iter().map(render).collect::<Vec<_>>().join(", ")
        ),
        NodeKind::Unary(inner, op) => match op {
            UnOp::Not => format!("not {}", render(inner)),
            UnOp::BitNot => format!("~{}", render(inner)),
            UnOp::Minus => format!("-{}", render(inner)),
            UnOp::Length => format!("#{}", render(inner)),
        },
        NodeKind::Binary(l, r, op) => format!("{} {} {}", render(l), op.name(), render(r)),
        NodeKind::If(ifb) => render_if(ifb),
        NodeKind::Match(m) => render_match(m),
        NodeKind::Do(blk, ..) => render_block(blk),
        NodeKind::Table(fields) => format!(
            "{{{}}}",
            fields
                .iter()
                .map(|f| match f {
                    Field::NameValue((name, _), value) => format!("{name}: {}", render(value)),
                    Field::KeyValue(k, v) => format!("[{}] = {}", render(k), render(v)),
                    Field::Value(v) => render(v),
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        NodeKind::Array(items) => {
            format!(
                "[{}]",
                items.iter().map(render).collect::<Vec<_>>().join(", ")
            )
        }
        other => format!("({other:?})"),
    }
}

fn render_path(path: &Path) -> String {
    match path {
        Path::Base((name, _)) => name.to_string(),
        Path::Expr(e) => render(e),
        Path::Chain(base, suffix) => match suffix {
            PathSuffix::Dot((name, _)) => format!("{}.{name}", render_path(base)),
            PathSuffix::Index(idx) => format!("{}[{}]", render_path(base), render(idx)),
            PathSuffix::Colon((name, _)) => format!("{}:{name}", render_path(base)),
            PathSuffix::TypeArgs(..) => format!("{}.<type(s)>", render_path(base)),
            PathSuffix::NilCheck(_) => format!("{}?", render_path(base)),
        },
    }
}

fn render_if(ifb: &If) -> String {
    // `IfClause` is (block, condition): `If.0` is the first `if`, `If.1` the
    // `elseif`s, and `If.2` the `else` block.
    let mut out = format!("if {} ", render(&ifb.0.1));
    out.push_str(&render_block(&ifb.0.0));
    out.push(' ');
    for clause in ifb.1.iter() {
        out.push_str(&format!("elseif {} ", render(&clause.1)));
        out.push_str(&render_block(&clause.0));
        out.push(' ');
    }
    if let Some(else_block) = &ifb.2 {
        out.push_str(&format!("else {} ", render_block(else_block)));
    }
    out.push_str("end");
    out
}

fn render_match(m: &Match) -> String {
    let mut out = format!("match {} then ", render(&m.0));
    for clause in m.1.iter() {
        out.push_str(&format!(
            "{} -> {}; ",
            render_pattern(&clause.0),
            render_block(&clause.1)
        ));
    }
    if let Some(else_block) = &m.2 {
        out.push_str(&format!("else {} ", render_block(else_block)));
    }
    out.push_str("end");
    out
}

fn render_pattern(pattern: &Pattern) -> String {
    match &pattern.0 {
        PatternTerm::Constant(cv) => render(cv),
        PatternTerm::Bind((name, _), _) => name.to_string(),
        PatternTerm::Type((name, _), _) => name.to_string(),
        other => format!("({other:?})"),
    }
}

#[inline]
/// The name of a declared name, off an `AttrName`, which is a name with its
/// attributes and its optional annotation, all inside a span.
fn attr_name(name: &AttrName) -> String {
    let (((name, _), _, _), _) = name;
    name.to_string()
}

fn render_block(block: &Block) -> String {
    block
        .0
        .iter()
        .map(|stmt| match &stmt.0 {
            NodeKind::Expr(e) => render(e),
            NodeKind::Call(callee, args) => format!(
                "{}({})",
                render(callee),
                args.iter().map(render).collect::<Vec<_>>().join(", ")
            ),
            NodeKind::Return(items, ..) => format!(
                "return {}",
                items.iter().map(render).collect::<Vec<_>>().join(", ")
            ),
            NodeKind::Define(names, values, ..) => format!(
                "local {} = {}",
                names.iter().map(attr_name).collect::<Vec<_>>().join(", "),
                values.iter().map(render).collect::<Vec<_>>().join(", ")
            ),
            NodeKind::TypeAlias((name, _), ty) => format!("type {name} = {ty}"),
            NodeKind::Assign(paths, values) => format!(
                "{} = {}",
                paths.iter().map(render_path).collect::<Vec<_>>().join(", "),
                values.iter().map(render).collect::<Vec<_>>().join(", ")
            ),
            NodeKind::If(ifb) => render_if(ifb),
            NodeKind::Match(m) => render_match(m),
            NodeKind::Do(blk, ..) => format!("do {} end", render_block(blk)),
            NodeKind::While(cond, blk, ..) => {
                format!("while {} {} end", render(cond), render_block(blk))
            }
            NodeKind::Break(..) => "break".into(),
            NodeKind::Continue(..) => "continue".into(),
            NodeKind::Empty => String::new(),
            other => format!("({other:?})"),
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Renders a type-level expression back to something readable, for a hover and
/// for the `Display` of an annotation. This is not the original text, which is
/// not reachable from here; it covers the subset above, which is all that can

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::{
        BangCollected, BangCollectedSource, Block, FuncBody, Node, NodeKind, Path, TypeDesc,
    };
    use duka_shared::{
        dtype::Type,
        errors::Position,
        types::{BinOp, UnOp},
        value::ConstValue,
    };

    fn expr(kind: NodeKind) -> Expr {
        Node(kind, Span::EMPTY)
    }
    fn stmt(kind: NodeKind) -> Stmt {
        Node(kind, Span::EMPTY)
    }
    /// An expression-level `and`, which is what a predicate clause is made of
    fn and(l: NodeKind, r: NodeKind) -> NodeKind {
        NodeKind::Binary(Box::new(expr(l)), Box::new(expr(r)), BinOp::And)
    }
    fn call_of_a_value_closure() -> NodeKind {
        NodeKind::Call(
            Box::new(expr(NodeKind::Function(empty_body()))),
            Box::new([expr(NodeKind::Literal(ConstValue::Int(1)))]),
        )
    }
    fn empty_body() -> FuncBody {
        FuncBody(
            Box::new([]),
            Box::new([]),
            None,
            Box::new([]),
            Box::new(Block::empty()),
        )
    }
    fn empty_block(stmts: Vec<Stmt>) -> Block {
        Block(stmts.into())
    }

    /// The type level is the expression language minus the constructs that only
    /// mean something at runtime.
    #[test]
    fn the_computable_forms_are_type_level() {
        for kind in [
            NodeKind::Literal(ConstValue::Int(1)),
            NodeKind::Literal(ConstValue::Bool(true)),
            NodeKind::Empty,
            NodeKind::VarArg,
            NodeKind::Access(Box::new(Path::Base(("T".into(), Span::EMPTY)))),
            NodeKind::Unary(
                Box::new(expr(NodeKind::Literal(ConstValue::Bool(true)))),
                UnOp::Not,
            ),
            NodeKind::Binary(
                Box::new(expr(NodeKind::Literal(ConstValue::Int(1)))),
                Box::new(expr(NodeKind::Literal(ConstValue::Int(2)))),
                BinOp::Add,
            ),
            and(
                NodeKind::Literal(ConstValue::Bool(true)),
                NodeKind::Literal(ConstValue::Bool(false)),
            ),
            NodeKind::TypeLit(TypeDesc::Pure(Type::Int)),
        ] {
            assert!(
                TypeLevelGate::first_offender(&expr(kind.clone())).is_none(),
                "{kind:?} should be type level"
            );
        }
    }

    /// The runtime-only forms are what the gate exists for: without it they
    /// reach the evaluator, which answers `any` instead of refusing.
    #[test]
    fn the_runtime_only_forms_are_refused() {
        for kind in [
            NodeKind::Function(empty_body()),
            NodeKind::BangCollected(BangCollected {
                name: "x".into(),
                source: BangCollectedSource::Raw("1".into()),
            }),
        ] {
            assert!(
                TypeLevelGate::first_offender(&expr(kind.clone())).is_some(),
                "{kind:?} should be refused at the type level"
            );
        }
    }

    /// The gate has to look inside, not just at the root: the offending node is
    /// usually buried under a call or a comparison.
    #[test]
    fn an_offender_is_found_below_the_root() {
        let deep = and(
            call_of_a_value_closure(),
            NodeKind::Literal(ConstValue::Bool(true)),
        );
        assert!(TypeLevelGate::first_offender(&expr(deep)).is_some());
    }

    /// A type-level `do` block carries type-level statements, and the gate has
    /// to hold those to the same subset as the expressions around them.
    #[test]
    fn a_type_level_block_is_judged_by_its_statements() {
        let good = expr(NodeKind::Do(
            Box::new(empty_block(vec![stmt(NodeKind::Expr(Box::new(expr(
                NodeKind::Literal(ConstValue::Int(1)),
            ))))])),
            false,
            None,
        ));
        assert_eq!(TypeLevelGate::first_offender(&good), None);

        // a nested function declaration is not something a type position evaluates
        let bad = expr(NodeKind::Do(
            Box::new(empty_block(vec![stmt(NodeKind::FunctionDecl(
                Path::Base(("f".into(), Span::EMPTY)),
                Default::default(),
                Box::new(empty_body()),
                false,
            ))])),
            false,
            None,
        ));
        assert!(TypeLevelGate::first_offender(&bad).is_some());
    }

    /// The span reported is the offending node's own, so the diagnostic lands
    /// on the construct and not on the whole annotation.
    #[test]
    fn the_reported_span_is_the_offending_node() {
        let at = |line| Span {
            start: Position {
                line,
                column: 1,
                at_char: 0,
            },
            end: Position {
                line,
                column: 9,
                at_char: 8,
            },
        };
        let buried = Node(
            and(
                NodeKind::Call(
                    Box::new(Node(NodeKind::Function(empty_body()), at(7))),
                    Box::new([expr(NodeKind::Literal(ConstValue::Int(1)))]),
                ),
                NodeKind::Literal(ConstValue::Bool(true)),
            ),
            at(1),
        );
        assert_eq!(
            TypeLevelGate::first_offender(&buried).map(|s| s.start.line),
            Some(7)
        );
    }
}
