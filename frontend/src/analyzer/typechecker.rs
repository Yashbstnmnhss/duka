use super::destructuring;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use duka_shared::attributes::Attributes;
    use duka_shared::constants::{MetaMethod, ccallish, cpar};
use duka_shared::{
    constants::csugar,
    dtype::{Constraint, FunctionType, ObjectId, Subject, Type, TypeVar},
    errors::{DukaErrorKind, DukaSemanticError, DukaSpannedError, Span},
    types::{BinOp, DukaAnalyzer, SourceInfo, UnOp},
    utils::{SymbolTableViewer, SymbolType},
    value::ConstValue,
};

use crate::analyzer::{BindingKind, BuiltinRules, GenericBinding, ModuleType, attributes};
use crate::solver;
use crate::{
    analyzer::{
        AnalyzerData, CallResults, InlineTypeFn, TypeFn, Visit, Visitor,
        eval::{EvalCtx, EvalCtxInit},
        modules::DukaSourceProvider,
        objects::{self, MethodLink, ObjectMethod, ObjectType},
        tyval::{TypeClosure, TypeValue},
    },
    parser::ast::{
        Attrs, Block, Destructuring, DukaChunk, Expr, Field, FuncBody, If, IfClause, Node,
        NodeKind, Param, ParamMode, Path, PathSuffix, Stmt, TypeDesc, TypeFnValue, TypeParam,
        WhereClause,
    },
};
use duka_shared::attributes::AttrReturns;
use duka_shared::docs::{ConstraintMeta, WhereMeta};

#[derive(Debug, Clone, PartialEq)]
pub struct TypeChecker;

impl DukaAnalyzer for TypeChecker {
    type InputType = DukaChunk;
    type InputData = AnalyzerData;
    type OutputData = AnalyzerData;

    fn analyze(
        &self,
        chunk: &Self::InputType,
        data: Self::InputData,
    ) -> (Self::OutputData, impl Iterator<Item = DukaSpannedError>) {
        self.analyze_with_modules(chunk, data, None)
    }
}

impl TypeChecker {
    /// Infers the value level shape of a module: the type of every `export`ed
    /// symbol plus the type `require` gives back to the importer
    ///
    /// `key` is the module's own name in the map, which is what every class it
    /// declares is stamped with: an id only indexes the table of the file that
    /// handed it out, so a class that leaves through a `require` has to say which
    /// table that is.
    pub fn analyze_module<'a>(
        &self,
        chunk: &'a DukaChunk,
        mut data: AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
        key: Option<&str>,
    ) -> (AnalyzerData, Vec<DukaSpannedError>, ModuleValue) {
        let source = Arc::new(chunk.source_info.clone());
        let (inferred, inferred_closures, inferred_do_values) =
            Self::infer_returns(chunk, &data, provider, key);
        let mut ctx = TypeCheckerCtx::new(source, &data, provider, key.map(Box::from));
        ctx.inferred_returns = inferred;
        ctx.inferred_closures = inferred_closures;
        ctx.inferred_do_values = inferred_do_values;
        ctx.mark_pipe_operands(chunk);
        chunk.visit(&mut ctx);
        let tail = chunk
            .block
            .0
            .iter()
            .rev()
            .find(|stmt| matches!(stmt.0, NodeKind::Return(..)))
            .and_then(|tail| ctx.infer_tail(tail));
        let mut links = std::mem::take(&mut ctx.links);
        let generic_bindings = std::mem::take(&mut ctx.generic_bindings);
        let type_at_span = std::mem::take(&mut ctx.type_at_span);
        links.sort_by_key(|l| (l.call_span, l.name_span, l.decl_span, l.owner));
        links.dedup();
        let value = ctx.module_value(tail);

        let (errors, backfills) = ctx.finish();
        for (id, span, ty, value) in backfills {
            match id {
                Some(id) => {
                    data.1.symbols.set_type_display(id, ty);
                    if let Some(value) = value {
                        data.1.symbols.set_type_value(id, Arc::new(value));
                    }
                }
                None => {
                    data.1.symbols.set_type_at_span(span, ty);
                    if let Some(value) = value {
                        data.1.symbols.set_type_value_at_span(span, Arc::new(value));
                    }
                }
            }
        }
        data.1.links = links;
        data.1.generic_bindings = generic_bindings;
        // merged rather than replaced: a module resolves its own types through
        // the ones its dependencies recorded, and a reader asking about a name
        // in this file is asking about the entry in this file
        data.1.type_at_span.extend(type_at_span);
        (data, errors.collect(), value)
    }

    fn infer_returns<'a>(
        chunk: &'a DukaChunk,
        data: &'a AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
        module: Option<&str>,
    ) -> (
        HashMap<Box<str>, InferredReturns>,
        HashMap<Span, InferredReturns>,
        HashMap<Span, Vec<Type>>,
    ) {
        let source = Arc::new(chunk.source_info.clone());
        let mut inferred: (
            HashMap<Box<str>, InferredReturns>,
            HashMap<Span, InferredReturns>,
            HashMap<Span, Vec<Type>>,
        ) = (HashMap::new(), HashMap::new(), HashMap::new());
        for _ in 0..MAX_INFER_ROUNDS {
            let mut collect =
                TypeCheckerCtx::new(source.clone(), data, provider, module.map(Box::from));

            collect.collect_mode = true;
            collect.inferred_returns = inferred.0.clone();
            collect.inferred_closures = inferred.1.clone();
            collect.inferred_do_values = inferred.2.clone();
            chunk.visit(&mut collect);
            if collect.collected_returns == inferred.0
                && collect.collected_closures == inferred.1
                && collect.collected_do_values == inferred.2
            {
                break;
            }
            inferred = (
                collect.collected_returns,
                collect.collected_closures,
                collect.collected_do_values,
            );
        }
        inferred
    }

    pub fn analyze_with_modules<'a>(
        &self,
        chunk: &'a DukaChunk,
        mut data: AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
    ) -> (AnalyzerData, impl Iterator<Item = DukaSpannedError>) {
        let source = Arc::new(chunk.source_info.clone());
        let (inferred, inferred_closures, inferred_do_values) =
            Self::infer_returns(chunk, &data, provider, None);
        let mut ctx = TypeCheckerCtx::new(source, &data, provider, None);
        ctx.inferred_returns = inferred;
        ctx.inferred_closures = inferred_closures;
        ctx.inferred_do_values = inferred_do_values;
        ctx.mark_pipe_operands(chunk);
        chunk.visit(&mut ctx);
        let mut links = std::mem::take(&mut ctx.links);
        let generic_bindings = std::mem::take(&mut ctx.generic_bindings);
        let type_at_span = std::mem::take(&mut ctx.type_at_span);
        links.sort_by_key(|l| (l.call_span, l.name_span, l.decl_span, l.owner));
        links.dedup();

        let (errors, backfills) = ctx.finish();
        for (id, span, ty, value) in backfills {
            match id {
                Some(id) => {
                    data.1.symbols.set_type_display(id, ty);
                    if let Some(value) = value {
                        data.1.symbols.set_type_value(id, Arc::new(value));
                    }
                }
                None => {
                    data.1.symbols.set_type_at_span(span, ty);
                    if let Some(value) = value {
                        data.1.symbols.set_type_value_at_span(span, Arc::new(value));
                    }
                }
            }
        }
        data.1.links = links;
        data.1.generic_bindings = generic_bindings;
        data.1.type_at_span.extend(type_at_span);

        (data, errors)
    }
}

type Multi = (Vec<Type>, bool);

/// What the `break` of a body carries, which is what the body is worth.
///
/// A `break` leaves the nearest `do` rather than the function, so it is where a
/// `do`'s value comes from.
///
/// It is written inside a conditional as readily as on its own, and a `break`
/// under an `if` or a `match` still leaves this body, so those are stepped into
/// too. A loop is not: a `break` under one leaves the loop, which is a different
/// body, and a nested `do` the writer broke out of is one whose value already
/// went somewhere else. A named `break` is the exception to both: a name reaches
/// past every body between it and the one it names, so a loop or a nested `do`
/// on the way there does not stop the search. `own` is the name of the body being
/// measured, and `direct` whether an unnamed `break` found here is still its own
/// rather than some body nested in between.
///
/// Every `break` is folded in rather than the first one read: they are the same
/// value by the time the block is worth anything, and a body that disagrees with
/// itself is worth the union of what it said.
///
/// A `break` that carries fewer values than another leaves `nil` in the rest,
/// because that is what the block hands back on that path -- a function's short
/// `return` leaves a slot merely unconstrained, which is a different thing.
fn break_types_of(checker: &mut TypeCheckerCtx<'_>, block: &Block, own: Option<&str>) -> Vec<Type> {
    let mut carried: Vec<Vec<Type>> = vec![];
    gather_breaks(checker, block, own, true, &mut carried);
    let widest = carried.iter().map(Vec::len).max().unwrap_or(0);
    let mut gathered = InferredReturns::default();
    for tys in &carried {
        let mut tys = tys.clone();
        tys.resize(widest, Type::Nil);
        gathered.merge(&tys);
    }
    gathered.types.into_vec()
}

fn gather_breaks(
    checker: &mut TypeCheckerCtx<'_>,
    block: &Block,
    own: Option<&str>,
    direct: bool,
    out: &mut Vec<Vec<Type>>,
) {
    for stmt in block.0.iter() {
        match &stmt.0 {
            NodeKind::Break(to, values, _) => {
                let mine = match to {
                    None => direct,
                    Some(name) => own == Some(name.0.as_str()),
                };
                if mine {
                    let (tys, _) = checker.infer_expr_list(values);
                    out.push(tys);
                }
            }
            // a `do` the desugarer invented opens no body of its own, so a
            // `break` in it leaves whatever encloses the fold
            NodeKind::Do(inner, ..) if !stmt.0.is_breakable_do() => {
                gather_breaks(checker, inner, own, direct, out)
            }
            // one that is a body keeps its own `break`s, but a name aimed past
            // it still has to be seen on the way out
            NodeKind::Do(inner, ..) => gather_breaks(checker, inner, own, false, out),
            // a `do` written where a value is expected is the same body, one
            // wrapper down: a `break` in it leaves that, not this
            NodeKind::Expr(inner) => {
                if let NodeKind::Do(block, ..) = &inner.0 {
                    gather_breaks(checker, block, own, false, out);
                }
            }
            // a loop is a body too, and takes no value of its own -- but a name
            // aimed past it still counts for whoever it names
            NodeKind::While(_, blk, ..)
            | NodeKind::ForNumeric(_, _, _, _, blk, _)
            | NodeKind::ForGeneric(_, _, blk, ..) => gather_breaks(checker, blk, own, false, out),
            NodeKind::If(if_) => {
                gather_breaks(checker, &if_.0.0, own, direct, out);
                for clause in if_.1.iter() {
                    gather_breaks(checker, &clause.0, own, direct, out);
                }
                if let Some(els) = &if_.2 {
                    gather_breaks(checker, els, own, direct, out);
                }
            }
            NodeKind::Match(m) => {
                for clause in m.1.iter() {
                    gather_breaks(checker, &clause.1, own, direct, out);
                }
                if let Some(els) = &m.2 {
                    gather_breaks(checker, els, own, direct, out);
                }
            }
            _ => {}
        }
    }
}

/// What a `do` block is worth, read from what its `break` carried.
///
/// The gathered reading is preferred because the block has been walked by the
/// time this runs; reading it direct is the fallback for a `do` no gather pass
/// reached.
fn do_value(checker: &mut TypeCheckerCtx<'_>, block: &Block, own: Option<&str>) -> Vec<Type> {
    if let Some(gathered) = checker.inferred_do_values.get(&do_span(block)) {
        return gathered.clone();
    }
    break_types_of(checker, block, own)
}

/// The class a name was written as, when the adapter lowered one under it.
///
/// The lowering gives the class to the analyzer as data, so the class is looked
/// for in the class table rather than through a name lookup: the lowering's own
/// local stands where the class's name is and answers to that name.
fn class_named(checker: &TypeCheckerCtx<'_>, name: &str) -> Option<Type> {
    let id = checker.objects.iter().position(|o| &*o.name == name)?;
    let o = &checker.objects[id];
    Some(Type::Object {
        id,
        name: o.name.clone(),
        base: o.base,
        args: Box::new([]),
        module: checker.module.clone(),
        mt: None,
    })
}

/// Where a `do` block was written, which is what its gathered value is keyed by.
///
/// A block has no span of its own, so this is the span of its last statement --
/// the same reading `body_span` gives a function body.
fn do_span(block: &Block) -> Span {
    block.0.last().map_or(Span::EMPTY, |stmt| stmt.1)
}

/// Where a body was written. A closure has no name of its own to file its
/// inferred returns under, so this is what it is keyed by instead.
fn body_span(body: &FuncBody) -> Span {
    body.4.0.last().map_or(Span::EMPTY, |stmt| stmt.1)
}

const MAX_INFER_ROUNDS: usize = 64;

/// The value level shape of a module, what an importer gets from `require`
#[derive(Debug, Clone)]
pub struct ModuleValue {
    /// `require` hands this type back to the importer
    pub ty: Type,
    /// types of the symbols the module `export`s
    pub members: HashMap<Box<str>, Type>,
    /// What an attribute written on the exported declaration says about it.
    ///
    /// `Type` has no room for this, and the type alone is not the whole of what
    /// an export is. `@declare` is the case that needs it: the export describes
    /// something the module does not build, so the importer has to be able to
    /// tell a promise from a value -- and the promise is the only thing there
    /// is to tell.
    pub attributes: HashMap<Box<str>, Attributes>,
}
impl Default for ModuleValue {
    fn default() -> Self {
        Self {
            ty: Type::Table(None, None, None),
            members: HashMap::new(),
            attributes: HashMap::new(),
        }
    }
}

use crate::analyzer::builtin::concept_holds;

struct TypeCheckerCtx<'a> {
    source: Arc<SourceInfo>,
    ret_stack: Vec<Option<Multi>>,
    types: Vec<HashMap<Box<str>, Type>>,
    facts: Vec<HashMap<Box<str>, Type>>,
    viewer: SymbolTableViewer<'a>,
    objects: &'a [ObjectType],
    /// Which class each instance method's body belongs to, so `self` is that
    /// class inside it.
    method_owner: HashMap<Span, ObjectId>,
    aliases: &'a [(Box<str>, TypeDesc)],
    type_fns: &'a [TypeFn],
    inline_type_fns: &'a [InlineTypeFn],
    userdata_types: &'a HashMap<Box<str>, Box<[(Box<str>, Type)]>>,
    /// What the builtins say about their own calls
    builtin_rules: &'a HashMap<Box<str>, BuiltinRules>,
    call_cache: Arc<Mutex<CallResults>>,
    closures: Arc<Mutex<Vec<TypeClosure>>>,
    modules: &'a HashMap<Box<str>, ModuleType>,
    /// The module this file is, when it is one. `None` for the entry.
    ///
    /// Every class this file declares is stamped with it, so a class that leaves
    /// through a `require` says which file's table its id indexes.
    module: Option<Box<str>>,
    provider: Option<&'a dyn DukaSourceProvider>,
    generic_fns: HashMap<Box<str>, GenericDecl>,
    links: Vec<MethodLink>,
    generic_bindings: Vec<(Span, Vec<GenericBinding>)>,
    errors: Vec<DukaSpannedError>,
    backfills: Vec<(Option<usize>, Span, Box<str>, Option<Type>)>,
    collect_mode: bool,
    ret_collect: Vec<InferredReturns>,
    finished_returns: Vec<InferredReturns>,
    collected_returns: HashMap<Box<str>, InferredReturns>,
    inferred_returns: HashMap<Box<str>, InferredReturns>,
    /// The same, for closures: one with no name of its own is keyed by where it
    /// was written, since `return`s inside it are just as inferable as a
    /// function statement's
    collected_closures: HashMap<Span, InferredReturns>,
    inferred_closures: HashMap<Span, InferredReturns>,
    /// The same, for a `do`: what its `break` carried, keyed by where the block
    /// was written
    ///
    /// A `do` is worth whatever its `break` carries, so the reading needs the
    /// block already walked -- the names a `break` reads are declared inside it.
    /// Inference reaches the `do` first, so this is gathered at the block's exit
    /// and read back, the way a function's returns are.
    collected_do_values: HashMap<Span, Vec<Type>>,
    inferred_do_values: HashMap<Span, Vec<Type>>,
    final_args: bool,
    /// What each expression was inferred to be, keyed by where it is written.
    ///
    /// A loop body is walked before its iterator expression in the checker's
    /// own order, and a generic `for` has to know what it iterates before it can
    /// name the variables. The expression has already been inferred by then, so
    /// it is read back from here rather than inferred a second time.
    expr_types: HashMap<Span, Type>,
    /// The calls that sit inside a `|>` operand.
    ///
    /// `data |> f(x)` is `f(data, x)`, and the data only arrives when the
    /// adapter folds the pipe -- which is after this pass. Checking the call as
    /// it is written would count one argument where the fold will count two, so
    /// these are left to the expression reading below, which does no arity check.
    pipe_operands: std::collections::HashSet<Span>,
    export_members: HashMap<Box<str>, Type>,
    /// what an attribute written on an `export`ed declaration says about it, so
    /// `ModuleValue` can hand the importer more than a type
    export_attributes: HashMap<Box<str>, Attributes>,
    /// what each name written as a member of a type resolved to, keyed by where
    /// it is written
    type_at_span: HashMap<Span, Arc<Type>>,
    /// Whether a member read is being recorded right now.
    ///
    /// Recording one resolves it, and resolving a type records the members in it,
    /// so the member being resolved is not in the table until it returns. Without
    /// this the same walk runs again over the same nodes and resolves that member
    /// a second time, for as long as it takes.
    recording_members: bool,
    /// The unknown type names already reported, so that a name reached twice --
    /// once building the signature and once checking the body -- is one mistake
    /// and not two.
    reported_unknown_types: Vec<(Box<str>, Span)>,
    /// What calls promised, held until the truth of the call is read.
    ///
    /// `not_nil(x)` answers whether `x` is not nil, so what it guarantees about
    /// `x` holds where that answer is known to be true -- `if not_nil(x) then`
    /// -- and not from the call onwards. Each entry carries the span of the
    /// call that promised it, so a condition picks up exactly what its own
    /// calls promised and nothing that was said somewhere else.
    pending: Vec<(Span, Constraint)>,
    /// Whether a declaration with no `?`/`!` written on it is non-nil.
    ///
    /// The parser has already read written annotations under this, so what is
    /// left for the checker is the type it inferred: in the relaxed mode a name
    /// that starts out holding a value may be given `nil` later, and a value it
    /// started with is not a promise that it never will be.
    default_nonnilable: bool,
    /// The conditions of each `if` currently being walked, the whole list at a
    /// time.
    ///
    /// A clause after the first is read with the clauses above it known to be
    /// false, and those conditions are the `if`'s own fields -- the clause hook
    /// only sees its own clause, so the list is kept here by the walk that does
    /// see the whole `if`, one entry per `if` currently open.
    clause_conds: Vec<Vec<Box<Expr>>>,
}

/// Finds the type names hiding in a type-level expression, so they get the same
/// check as the ones written in the plain type shapes. The derived visitor
/// already walks the whole expression; all it stops short of is the `TypeDesc`
/// inside a `TypeLit`, which is the one place a type can be written there.
struct TypeNameWalk<'ctx, 'a> {
    ctx: &'ctx mut TypeCheckerCtx<'a>,
    bound: Vec<Box<str>>,
}

/// Checks the names inside one type.
///
/// `TypeLit` is the one place a walk cannot go on its own: a type written as an
/// expression wraps its types in `TypeLit`, and marking that field as visited would
/// make a type refer to itself through the annotation that mentions it. So the
/// step is taken here instead -- the type inside the wrapper is checked, and the
/// wrapper is left alone.
impl Visitor for TypeNameWalk<'_, '_> {
    fn visit_node(&mut self, expr: &Expr) {
        if let NodeKind::TypeLit(td) = &expr.0 {
            self.ctx.check_type_names_with(td, &mut self.bound);
        }
    }
}

/// Every name written as a member inside `ty`, with the span it is written at
/// and the type of the path that ends there.
///
/// `RequireType("m").Alias.y` yields the path ending at `.Alias` and the one
/// ending at `.y`, because those are two references and a reader asking about
/// either one is asking about a different type.
fn collect_type_members(ty: &TypeDesc, out: &mut Vec<(Span, TypeDesc)>) {
    struct Walk<'a>(&'a mut Vec<(Span, TypeDesc)>);
    impl Visitor for Walk<'_> {
        fn visit_node(&mut self, expr: &Expr) {
            walk_type_expr(expr, self.0);
        }
    }
    let mut walk = Walk(out);
    ty.visit(&mut walk);
}

/// Steps into one type-level expression.
///
/// `TypeDesc::Expr` is the only shape whose contents are nodes rather than data,
/// so this is where a type written as an expression is walked. The walk is
/// spelled out over the node kinds a type-level expression may hold rather than
/// taken from the derived one, because `a.b.c` nests: `type_access` rebuilds the
/// base of every dot as a path, and a path wraps what it reads through in
/// `TypeLit` -- the one field the derived visitor stops short of, precisely so a
/// type cannot refer to itself through the annotation that mentions it. Every
/// dot before the last is therefore found here rather than by the walk.
fn walk_type_expr(expr: &Expr, out: &mut Vec<(Span, TypeDesc)>) {
    match &expr.0 {
        NodeKind::TypeLit(td) => collect_type_members(td, out),
        NodeKind::Access(path) => out.extend(member_prefixes(path)),
        NodeKind::Call(callee, args) => {
            walk_type_expr(callee, out);
            args.iter().for_each(|arg| walk_type_expr(arg, out));
        }
        NodeKind::Unary(inner, _) => walk_type_expr(inner, out),
        NodeKind::Binary(l, r, _) => {
            walk_type_expr(l, out);
            walk_type_expr(r, out);
        }
        _ => (),
    }
}

/// The prefix of a member read that ends at each of its members.
///
/// A suffix is what makes a path a chain, so the prefixes are built by walking
/// them left to right and stopping a copy at each `.name`: that copy is the type
/// a reader asking about that name means.
fn member_prefixes(path: &Path) -> Vec<(Span, TypeDesc)> {
    let mut suffixes = vec![];
    let mut head = path;
    while let Path::Chain(base, suffix) = head {
        suffixes.push(suffix);
        head = base.as_ref();
    }
    // a bare name is not a member read, however much like one it looks
    if suffixes.is_empty() {
        return vec![];
    }
    let mut prefix = head.clone();
    let mut out = vec![];
    for suffix in suffixes {
        prefix = prefix.clone() + suffix.clone();
        if let PathSuffix::Dot(name) = suffix {
            out.push((
                name.1,
                TypeDesc::Expr(Box::new(Node(
                    NodeKind::Access(Box::new(prefix.clone())),
                    name.1,
                ))),
            ));
        }
    }
    out
}

/// What the returns of a function without a return annotation work out to.
/// position rather than one after another: a function that returns `a` on one
/// path and `a, 1` on another returns `(int,)`, not `(int, int)`.
#[derive(Debug, Clone, PartialEq)]
struct InferredReturns {
    types: Box<[Type]>,
    /// The fewest values any single `return` handed back, against the most any
    /// of them did. A gap between the two means the tail is not always filled.
    shortest: usize,
    longest: usize,
    var_arg: bool,
}

impl Default for InferredReturns {
    fn default() -> Self {
        Self {
            types: Box::default(),
            shortest: usize::MAX,
            longest: 0,
            var_arg: false,
        }
    }
}

impl InferredReturns {
    /// Folds one `return` into what has been collected so far. A position this
    /// one leaves out keeps whatever the others say, and a position the others
    /// leave out stays unconstrained.
    fn merge(&mut self, incoming: &[Type]) {
        self.longest = self.longest.max(incoming.len());
        self.shortest = self.shortest.min(incoming.len());
        self.var_arg = self.shortest < self.longest;
        let mut types = self.types.to_vec();
        types.resize(self.longest, Type::Any);
        for (slot, ty) in types.iter_mut().zip(incoming) {
            if *slot == Type::Any {
                *slot = ty.clone();
            } else if *slot != *ty {
                *slot = slot.clone() | ty.clone();
            }
        }
        self.types = types.into_boxed_slice();
    }
}

impl<'a> TypeCheckerCtx<'a> {
    fn new(
        source: Arc<SourceInfo>,
        data: &'a AnalyzerData,
        provider: Option<&'a dyn DukaSourceProvider>,
        module: Option<Box<str>>,
    ) -> Self {
        Self {
            source,
            ret_stack: vec![None],
            types: vec![HashMap::new()],
            facts: vec![HashMap::new()],
                viewer: SymbolTableViewer::new(&data.1.symbols),
                objects: &data.1.objects,
                method_owner: data
                    .1
                    .objects
                    .iter()
                    .enumerate()
                    .flat_map(|(id, o)| {
                        o.methods
                            .iter()
                            .filter(|m| !m.is_static && m.body_span != Span::EMPTY)
                            .map(move |m| (m.body_span, id))
                    })
                    .collect(),
                aliases: &data.1.aliases,
            type_fns: &data.1.type_fns,
            inline_type_fns: &data.1.inline_type_fns,
            userdata_types: &data.1.userdata_types,
            builtin_rules: &data.1.builtin_rules,
            call_cache: data.1.call_cache.clone(),
            closures: data.1.closures.clone(),
            modules: &data.1.modules,
            module,
            provider,
            generic_fns: HashMap::new(),
            generic_bindings: vec![],
            links: vec![],
            errors: vec![],
            backfills: vec![],
            collect_mode: false,
            ret_collect: vec![],
            finished_returns: vec![],
            collected_returns: HashMap::new(),
            inferred_returns: HashMap::new(),
            collected_closures: HashMap::new(),
            inferred_closures: HashMap::new(),
            collected_do_values: HashMap::new(),
            inferred_do_values: HashMap::new(),
            final_args: true,
            expr_types: HashMap::new(),
            pipe_operands: std::collections::HashSet::new(),
            export_members: HashMap::new(),
            export_attributes: HashMap::new(),
            type_at_span: HashMap::new(),
            recording_members: true,
            reported_unknown_types: Vec::new(),
            pending: Vec::new(),
            default_nonnilable: data.0.default_nonnilable,
            clause_conds: Vec::new(),
        }
    }

    #[inline]
    fn finish(
        self,
    ) -> (
        impl Iterator<Item = DukaSpannedError> + use<>,
        Vec<(Option<usize>, Span, Box<str>, Option<Type>)>,
    ) {
        (self.errors.into_iter(), self.backfills)
    }

    #[inline]
    fn err(&mut self, v: DukaSemanticError, span: Span) {
        self.err_related(v, span, [])
    }

    /// The same statement can be read more than once in one pass (once by the
    /// expression reading above, once by the arm that owns it), and an
    /// identical diagnostic twice tells the reader nothing new.
    fn push_error(&mut self, error: DukaSpannedError) {
        if !self.errors.contains(&error) {
            self.errors.push(error);
        }
    }

    /// Reports a diagnostic that is about more than the one place it points at.
    ///
    /// A type mismatch is not only about the value that was found but about the
    /// type it was asked to be, and a name that was never declared is about
    /// whatever scope was supposed to have it. A reader cannot act on any of
    /// those from the primary span alone, so the other places travel with it.
    fn err_related<const N: usize>(
        &mut self,
        v: DukaSemanticError,
        span: Span,
        related: [(Box<str>, Span); N],
    ) {
        if self.collect_mode {
            return;
        }
        self.push_error(DukaSpannedError {
            level: Default::default(),
            kind: v.into(),
            span,
            related: related.into(),
            source_info: self.source.clone(),
        });
    }

    fn lookup_type(&self, name: &str) -> Option<Type> {
        let declared: Type = self.declared_type(name)?;
        // a fact that contradicts the declaration is not a narrowing
        self.fact(name)
            .filter(|t| objects::accepts(&declared, t, self.objects))
            .or(Some(declared))
    }

    fn declared_type(&self, name: &str) -> Option<Type> {
        for frame in self.types.iter().rev() {
            if let Some(t) = frame.get(name) {
                return Some(t.clone());
            }
        }

        // a class is a type as well as a value: `local a: A` names the class
        if let Some(ty) = class_named(self, name) {
            return Some(ty);
        }

        let symbol = self.viewer.lookup_anywhere(name)?;
        match &symbol.symbol_type {
            SymbolType::Constant(cv) => Some(cv.type_of()),
            // A name that stands for a shape *is* the shape: the alias was
            // declared once, and the table it was declared into holds what the
            // name means.
            SymbolType::TypeAlias(id) => alias_type(&self.aliases.get(*id)?.1),
            _ => symbol.ty.as_deref().cloned(),
        }
    }

    /// The narrowest type the flow has established for `name`, if any.
    fn fact(&self, name: &str) -> Option<Type> {
        self.facts
            .iter()
            .rev()
            .find_map(|frame| frame.get(name).cloned())
    }

    fn establish(&mut self, name: &str, narrowed: Type) {
        if let Some(frame) = self.facts.last_mut() {
            frame.insert(name.into(), narrowed);
        }
    }

    /// Drops what the flow knows about a name, which is what writing to it does.
    fn forget(&mut self, name: &str) {
        for frame in self.facts.iter_mut() {
            frame.remove(name);
        }
    }

    fn declare(&mut self, name: &str, span: Span, ty: Type) {
        self.declare_with_display(name, span, ty.clone(), ty);
    }

    /// How a type is written back to a reader.
    ///
    /// A type that a name stands for is shown by that name: an alias is how the
    /// language says the shape, and spelling it out everywhere hides it.
    fn display_type(&self, ty: &Type) -> String {
        match self.alias_name(ty) {
            Some(name) => name.to_owned(),
            None => ty.to_string(),
        }
    }

    /// The alias a type is the expansion of, if one is worth showing.
    ///
    /// Only a union is read back this way. A one-word alias adds nothing to the
    /// type it names, and picking one for `int` would hide the type instead of
    /// naming it.
    fn alias_name(&self, ty: &Type) -> Option<&str> {
        if !matches!(ty, Type::Union(parts) if parts.len() > 1) {
            return None;
        }
        self.aliases.iter().rev().find_map(|(name, desc)| {
            (alias_type(desc).as_ref() == Some(ty)).then_some(name.as_ref())
        })
    }

    /// Names the variables of a generic `for` from what it iterates.
    ///
    /// The body's frame is already on top when this is called, so the names land
    /// where the body reads them. A loop over an iterator has no answer here and
    /// stays as the scope analyzer left it.
    fn declare_for_vars(&mut self, paths: &[Path], exprs: &[Expr]) {
        let Some(iter) = exprs.first() else {
            return;
        };
        let Some(ty) = self.expr_types.get(&iter.1).cloned() else {
            return;
        };
        let Some(tys) = for_var_types(&ty, paths.len()) else {
            return;
        };
        for (path, ty) in paths.iter().zip(tys) {
            if let Path::Base((name, span)) = path {
                self.declare(name, *span, ty);
            }
        }
    }

    /// Declares a name with one type for the checker and another for a reader.
    ///
    /// An inferred local is declared nilable in the relaxed mode so it can take
    /// `nil` later, but that is a statement about the whole life of the name.
    /// What it holds where it is declared is the value that was put in it --
    /// `local b = x` after `x ~= nil` starts out holding a string, and showing
    /// `string?` there describes a possibility that has not happened yet.
    fn declare_with_display(&mut self, name: &str, span: Span, ty: Type, display: Type) {
        let st = (!self.collect_mode).then(|| display.to_string().into_boxed_str());
        // assigning ends every fact about what the name held
        for frame in self.facts.iter_mut() {
            frame.remove(name);
        }
        self.types
            .last_mut()
            .expect("there must be a type frame")
            .insert(name.into(), ty.clone());
        if let Some(st) = st {
            let id = self.viewer.table().symbol_at_span(span).map(|s| s.id);
            self.backfills.push((id, span, st, Some(ty)));
        }
    }

    /// Declares a name whose type was read from what was put in it.
    ///
    /// A written annotation has already been read under the config's nilable
    /// default by the parser -- `!` and `?` included -- so it is declared as it
    /// is. An inferred type was read from one value, and the relaxed mode lets
    /// the name hold `nil` later: the declaration is nullable.
    ///
    /// `nil_is_any` is for the value that came from a literal `nil` or from
    /// nothing at all. There is no type to read off such a value, so the name
    /// starts as `any`; a `nil` the flow narrowed a value down to is different,
    /// and is kept as the `nil` it was proved to be.
    fn declare_inferred(&mut self, name: &str, span: Span, ty: Type, nil_is_any: bool) {
        let value = if nil_is_any && ty == Type::Nil {
            Type::Any
        } else {
            ty.clone()
        };
        // `nil` has no type to declare: writing it into a name says the name
        // can hold a nil, not that it can hold nothing else. The value is still
        // what the flow reads until something is written to the name.
        let declared = if value == Type::Nil {
            Type::Any
        } else {
            value.clone()
        };
        let declared = if self.default_nonnilable {
            declared
        } else {
            declared.nilable()
        };
        self.declare_with_display(name, span, declared, value.clone());
        // the declaration allows `nil` later, but what was put in it is what
        // the flow reads until something is written to the name
        if !self.default_nonnilable || value == Type::Nil {
            self.establish(name, value);
        }
    }

    /// Declares each name a destructuring binds, with the type that value has at
    /// that place in the pattern.
    fn declare_destructured(&mut self, pattern: &Destructuring, value: &Type) {
        for bind in destructuring::bind(pattern, value) {
            if let destructuring::Bind::Named(name, found) = bind {
                self.declare_inferred(&name.0, name.1, found.unwrap_or(Type::Any), true);
            }
        }
    }
    /// Declare parameters' type in function body
    fn declare_params(&mut self, body: &FuncBody) {
        // A parameter's annotation may name a type parameter of the function
        // itself, and that name means `Type::Param` rather than a lookup that
        // happens to fail. Without this the annotation of `x: T` in
        // `function f<T>(x: T)` resolved to nothing and quietly became `any`.
        let names: Vec<&str> = body
            .1
            .iter()
            .map(|TypeParam((n, _), ..)| n.as_str())
            .collect();
        for param in body.0.iter() {
            match param {
                Param::Typed((name, span), ty) => {
                    let ty = normalize_generic_names(ty, &names);
                    self.check_type_names(&ty);
                    let ty = self.resolve_type(&ty);
                    self.declare(name, *span, ty)
                }
                Param::Name((name, span)) => self.declare_inferred(name, *span, Type::Any, true),
                Param::Var(..) | Param::Pack(..) => {
                    // `VarArg` is `...`, a pack lives at the evaluation layer
                }
            }
        }
    }

    fn resolve_type(&mut self, ty: &TypeDesc) -> Type {
        self.resolve_type_and_constraints(ty).0
    }

    /// A type, and what evaluating it guaranteed.
    ///
    /// The guarantees come out because a type function contributes them on the
    /// way to its answer, and a caller that only wants the answer has no reason
    /// to throw them away -- `ensure` is exactly such a caller.
    fn resolve_type_and_constraints(&mut self, ty: &TypeDesc) -> (Type, Vec<Constraint>) {
        let contributed = Rc::new(RefCell::new(Vec::new()));
        let out = self.resolve_type_with(ty, contributed.clone());
        (out, std::mem::take(&mut *contributed.borrow_mut()))
    }

    fn resolve_type_with(
        &mut self,
        ty: &TypeDesc,
        contributed: Rc<RefCell<Vec<Constraint>>>,
    ) -> Type {
        // every annotation reaches here, whatever else reads it, so this is the
        // one place where a member written inside a type is known to have been
        // resolved rather than parsed
        self.record_type_members(ty);
        match ty {
            TypeDesc::Pure(t) => t.clone(),
            other => {
                let init = EvalCtxInit {
                    source: self.source.clone(),
                    contributed,
                    viewer: self.viewer.clone(),
                    type_fns: self.type_fns,
                    inline_type_fns: self.inline_type_fns,
                    objects: self.objects,
                    aliases: self.aliases,
                    results: self.call_cache.clone(),
                    closures: self.closures.clone(),
                    modules: Some(self.modules),
                    provider: self.provider,
                    report_errors: true,
                    module: self.module.clone(),
                };
                let mut hook = |t: &TypeDesc| match t {
                    TypeDesc::TypeOf(expr, _) => Some(TypeValue::Type(self.infer_expr(expr))),
                    TypeDesc::Named(name, _) => self.lookup_type(name).map(TypeValue::Type),
                    _ => None,
                };
                let mut ev = EvalCtx::new(init).with_hook(Some(&mut hook));
                let value = ev.eval_type(other);
                let provided = std::mem::take(&mut ev.provided);
                ev.contributed.borrow_mut().extend(provided);
                let r = value.to_type();
                let errs = std::mem::take(&mut ev.errors);
                self.errors.extend(errs);
                r
            }
        }
    }

    /// Records what every name written as a member of `ty` resolved to.
    ///
    /// `RequireType("m").Alias` is a reference written where a type is expected:
    /// it names something, here usually something in another file, and binds
    /// nothing. The symbol table cannot answer for it -- there is no symbol at a
    /// type position -- so the answer is kept next to where the name is written,
    /// which is what a reader asking "what is this" has to go on.
    ///
    /// The whole point is to be one reading of a member rather than two: the
    /// answer recorded here is the one the annotation itself was built from, not
    /// a second evaluation that could disagree with it.
    fn record_type_members(&mut self, ty: &TypeDesc) {
        if !self.recording_members {
            return;
        }
        let mut found = vec![];
        collect_type_members(ty, &mut found);
        // Resolving a member resolves a type, which comes back here. Without the
        // flag that is the same walk over the same nodes again, and the member
        // being resolved right now is not recorded until it returns -- so it
        // would resolve itself forever.
        self.recording_members = false;
        for (span, member) in found {
            if self.type_at_span.contains_key(&span) {
                continue;
            }
            let resolved = self.resolve_type(&member);
            self.type_at_span.insert(span, Arc::new(resolved));
        }
        self.recording_members = true;
    }

    fn err_opt_related(
        &mut self,
        v: DukaSemanticError,
        span: Span,
        related: Option<(Box<str>, Span)>,
    ) {
        if self.collect_mode {
            return;
        }
        self.push_error(DukaSpannedError {
            level: Default::default(),
            kind: v.into(),
            span,
            related: related.into_iter().collect(),
            source_info: self.source.clone(),
        });
    }

    /// Reports a type name that resolves to nothing.
    ///
    /// A name like this used to answer `any`, which is worse than useless: every
    /// bound and every obligation then passes, and the mistake only surfaces
    /// much later as a mismatch against something unrelated.
    ///
    /// Asked of every written annotation, a `local`'s included: a typo does not
    /// become correct because the name is on a local. The evaluator cannot make
    /// the call itself, because it is also reached from speculative places with
    /// partial environments, where a name it cannot see is a parameter that has
    /// not been bound yet rather than a mistake.
    fn check_type_names(&mut self, td: &TypeDesc) {
        self.check_type_names_with(td, &mut Vec::new());
    }

    /// `bound` is what a `type fn(x)` lambda has named so far: a name it binds
    /// is not a type name, so `type fn(x) x?` is not a mistake.
    fn check_type_names_with(&mut self, td: &TypeDesc, bound: &mut Vec<Box<str>>) {
        match td {
            TypeDesc::Pure(_) => {}
            TypeDesc::Named(name, span) | TypeDesc::Splice(name, span) => {
                self.check_type_name(name, *span, bound);
            }
            TypeDesc::Generic(name, args, span) => {
                self.check_type_name(name, *span, bound);
                for arg in args.iter() {
                    self.check_type_names_with(arg, bound);
                }
            }
            TypeDesc::Array(inner) => {
                if let Some(inner) = inner {
                    self.check_type_names_with(inner, bound);
                }
            }
            TypeDesc::Table(key, value) => {
                if let Some(key) = key {
                    self.check_type_names_with(key, bound);
                }
                if let Some(value) = value {
                    self.check_type_names_with(value, bound);
                }
            }
            TypeDesc::Union(parts) | TypeDesc::TypeTuple(parts, _) => {
                for part in parts.iter() {
                    self.check_type_names_with(part, bound);
                }
            }
            TypeDesc::NonNil(inner) | TypeDesc::Nilable(inner) | TypeDesc::Rec(inner) => {
                self.check_type_names_with(inner, bound);
            }
            TypeDesc::TypeTable(fields) => {
                for field in fields.iter() {
                    self.check_type_names_with(&field.ty, bound);
                }
            }
            TypeDesc::Function(Some(f)) => {
                for param in f.params.iter() {
                    self.check_type_names_with(param, bound);
                }
                for ret in f.returns.iter() {
                    self.check_type_names_with(ret, bound);
                }
            }
            TypeDesc::Function(None) => {}
            TypeDesc::FnLit(body) => {
                let outer = bound.len();
                for param in body.0.iter() {
                    match param {
                        Param::Typed((name, _), ty) => {
                            self.check_type_names_with(ty, bound);
                            bound.push(name.clone().into_boxed_str());
                        }
                        Param::Name((name, _))
                        | Param::Pack((name, _), _)
                        | Param::Var(_, Some((name, _))) => {
                            bound.push(name.clone().into_boxed_str())
                        }
                        Param::Var(_, None) => {}
                    }
                }
                if let Some(ret) = &body.2 {
                    for ty in ret.tys.iter() {
                        self.check_type_names_with(ty, bound);
                    }
                }
                bound.truncate(outer);
            }
            TypeDesc::Expr(expr) | TypeDesc::TypeOf(expr, _) => {
                let mut walk = TypeNameWalk {
                    ctx: self,
                    bound: std::mem::take(bound),
                };
                expr.visit(&mut walk);
                *bound = walk.bound;
            }
        }
    }

    fn check_type_name(&mut self, name: &str, span: Span, bound: &[Box<str>]) {
        if bound.iter().any(|b| b.as_ref() == name) || self.is_known_type_name(name) {
            return;
        }
        let key: Box<str> = name.into();
        if self.reported_unknown_types.contains(&(key.clone(), span)) {
            return;
        }
        self.reported_unknown_types.push((key.clone(), span));
        self.err(DukaSemanticError::UnknownType(key), span);
    }

    fn is_known_type_name(&self, name: &str) -> bool {
        if self.lookup_type(name).is_some() {
            return true;
        }
        if self.type_fns.iter().any(|f| &*f.name == name)
            || self.inline_type_fns.iter().any(|f| &*f.name == name)
            || self.userdata_types.contains_key(name)
        {
            return true;
        }
        self.viewer.lookup_anywhere(name).is_some_and(|sym| {
            matches!(
                sym.symbol_type,
                SymbolType::TypeFunction(_)
                    | SymbolType::TypeBuiltin
                    | SymbolType::TypeAlias(_)
                    | SymbolType::ObjectClass(_)
            )
        })
    }
    /// Runs a `where` list, top to bottom, so a clause can see the one above it.
    ///
    /// This is the declaration pass. The type parameters are still
    /// `Type::Param` here, so a bound or a concept that mentions one cannot be
    /// decided: that is not a failure, it is an obligation, and it is recorded
    /// for the call site to discharge once the arguments are known. A concept
    /// that answers `false` about types nothing is still waiting on *is* a
    /// failure and is reported here.
    ///
    /// A `type V = E` is put in the type frame, which is what makes `: V` in the
    /// return annotation and `V` in the body resolve to the same thing. It is put
    /// there rather than rewritten into a `Type::Param` because the value is
    /// already the answer: `V` is `T`, not a new variable that happens to equal
    /// one.
    fn run_where(&mut self, clauses: &[WhereClause], params: &[Box<str>]) {
        for clause in clauses {
            match clause {
                WhereClause::Bound((_, _), bound, _) => {
                    // Checked and normalised the way a clause's other arm is:
                    // an unknown name is reported rather than widened to `any`,
                    // and the function's own type parameters are read as
                    // parameters instead of as names the evaluator cannot see.
                    //
                    // The bound itself is not decided here and nothing is recorded
                    // about it. A bound that still mentions a type parameter cannot
                    // be decided at the declaration -- nothing is known yet -- and
                    // that is not a failure. The clauses travel with the declaration
                    // in `GenericDecl`, and `discharge_where` walks them again at
                    // each call once the arguments are solved. That is where a bound
                    // is checked, and it is checked from the clause rather than from
                    // a copy taken here, so there is one reading of a bound instead
                    // of two to keep in step.
                    self.check_type_names(bound);
                    self.resolve_type(&normalize_generic_names(
                        bound,
                        &params.iter().map(|n| n.as_ref()).collect::<Vec<&str>>(),
                    ));
                }
                WhereClause::Concept(cond, span) => {
                    if !self.concept_truth(cond, *span, params) {
                        self.err(DukaSemanticError::WhereConceptFailed, *span);
                    }
                }
                WhereClause::Bind(names, value, _) => {
                    // A name in a clause is checked the way a name in a
                    // signature is, before it is resolved.
                    //
                    // It was not, and the difference was invisible: an
                    // unresolved name resolves to `any` rather than failing, so
                    // `where type C = Undeclared` bound `C` to `any`, and that
                    // `any` went on to stand for `C` in the return annotation
                    // above and in every value read out of it. Nothing failed
                    // because there was never a check to fail -- `check_type_names`
                    // was reached from the signature and not from here.
                    self.check_type_names(value);
                    // The clause's own type parameters are names the same way, and
                    // `resolve_type` cannot see them through the evaluator, so they
                    // are named here exactly as a signature names them.
                    let ty_par_names = params.iter().map(|n| n.as_ref()).collect::<Vec<&str>>();
                    let ty = self.resolve_type(&normalize_generic_names(value, &ty_par_names));
                    // A pattern on the left of a `type` clause reads the same way
                    // anywhere else it appears: each name takes what sits at its
                    // own place, so `type [C, _] = [V, T]` gives `C` the type `V`
                    // rather than the whole `[V, T]`.
                    for bind in destructuring::bind(names, &ty) {
                        if let destructuring::Bind::Named(name, found) = bind {
                            self.declare(&name.0, name.1, found.unwrap_or(ty.clone()));
                        }
                    }
                    // A clause is a declaration site, so a name introduced here
                    // has to exist before the span that uses it is resolved --
                    // which it does, since this is the same frame the body reads.
                    if let Some(first) = destructuring::names(names).first() {
                        debug_assert!(
                            self.lookup_type(&first.0).is_some(),
                            "a where binding must be in scope once its clause has run"
                        );
                    }
                }
            }
        }
    }

    /// A concept read for its truth. Duka's own reading: `nil` and `false` fail
    /// and everything else answers, because a concept is written over types and
    /// almost no type is a boolean.
    ///
    /// A concept that mentions one of this function's own type parameters cannot
    /// be read here at all -- nothing is known yet -- so it is recorded as an
    /// obligation and answered as if it held, rather than being called false. The
    /// mention is looked for in the concept itself and not in its answer, because
    /// `T == int` answers `false` rather than saying anything about `T`.
    fn concept_truth(&mut self, cond: &Expr, _span: Span, params: &[Box<str>]) -> bool {
        if type_expr_names_type_param(cond, params) {
            // Answered as if it held, because it cannot be read yet: `T == int`
            // answers `false` rather than saying anything about `T`. Nothing is
            // recorded, because the clause travels with the declaration and
            // `discharge_where` reads it again at each call, with the solution
            // substituted -- which is the only place the question has an answer.
            return true;
        }
        let ty = self.resolve_type(&TypeDesc::Expr(Box::new(cond.clone())));
        concept_holds(&ty)
    }

    /// What a `: ensure C(T)` promised, as constraints about named type parameters.
    ///
    /// Read here rather than at each call because this is the one place the type
    /// parameters still have their names; at the call they have solutions, and a
    /// constraint about "the thing `t` was" cannot be phrased in terms of those.
    fn ensure_of(body: &FuncBody) -> Option<Box<Expr>> {
        body.2
            .as_ref()
            .and_then(|r| r.ensure.as_ref().map(|(e, _)| e.clone()))
    }

    /// Puts one guarantee into the flow.
    ///
    /// The subject has already been named by the time this sees it: a builtin counts
    /// from where it was called, and the evaluator turns that count into the name of
    /// a type parameter when the argument was one. So a fact is `one guarantee about
    /// one name`, and nothing here has to know which argument anyone passed.
    fn apply(&mut self, c: &Constraint) {
        fn name(s: &Subject) -> Option<&Box<str>> {
            match s {
                Subject::Local(n) | Subject::TypeParam(n) => Some(n),
                Subject::Arg(_) => None,
            }
        }
        let narrow = |me: &mut Self, p: &str, t: Type| {
            let declared = me.declared_type(p).unwrap_or(Type::Any);
            // a fact narrower than the declaration is a narrowing; one that
            // asks for more than the declaration allows is not
            if objects::accepts(&declared, &t, me.objects) {
                me.establish(p, t);
            }
        };
        match c {
            Constraint::IsNot(s, t) => {
                let Some(p) = name(s).cloned() else { return };
                let declared = self.declared_type(&p).unwrap_or(Type::Any);
                // excluding one literal at a time has to build on what was
                // already excluded: `x ~= true and x ~= nil` is one value left,
                // not whichever of the two exclusions was read last
                let base = self
                    .fact(&p)
                    .filter(|f| objects::accepts(&declared, f, self.objects))
                    .unwrap_or_else(|| declared.clone());
                narrow(self, &p, without(&base, t));
            }
            Constraint::Is(s, t) | Constraint::Instance(s, t) => {
                if let Some(p) = name(s) {
                    narrow(self, &p, t.clone());
                }
            }
            // the subject is a sub type of what the constraint carries, so that
            // super type is what the flow can hold on to
            Constraint::Sub(s, t) => {
                if let Some(p) = name(s) {
                    narrow(self, &p, t.clone());
                }
            }
            // a member that is asked for picks out of a union the members that
            // have it, and a type that cannot have it is not inhabited at all
            Constraint::Has(s, t) => {
                let Some(p) = name(s).cloned() else { return };
                let declared = self.declared_type(&p).unwrap_or(Type::Any);
                let kept: Vec<Type> = match &declared {
                    Type::Union(us) => us
                        .iter()
                        .filter(|m| has_member(m, t, self.objects))
                        .cloned()
                        .collect(),
                    other if has_member(other, t, self.objects) => vec![other.clone()],
                    _ => vec![],
                };
                narrow(self, &p, Type::from_union_kept(kept));
            }
            // nothing can be it, so nothing from here on is reached
            Constraint::Never(s) => {
                if let Some(p) = name(s) {
                    narrow(self, &p, Type::Never);
                }
            }
            Constraint::And(cs) => {
                for c in cs {
                    self.apply(c);
                }
            }
            _ => {}
        }
    }

    fn resolve_module_type(&self, name: &str) -> Option<&'a ModuleType> {
        crate::analyzer::modules::resolve_module_type(
            self.modules,
            name,
            self.source.name.path(),
            self.provider?,
        )
    }

    fn resolve_display(&mut self, td: &TypeDesc) -> Option<String> {
        let init = EvalCtxInit {
            source: self.source.clone(),
            contributed: Rc::new(RefCell::new(Vec::new())),
            viewer: self.viewer.clone(),
            type_fns: self.type_fns,
            inline_type_fns: self.inline_type_fns,
            objects: self.objects,
            aliases: self.aliases,
            results: self.call_cache.clone(),
            closures: self.closures.clone(),
            modules: Some(self.modules),
            provider: self.provider,
            report_errors: false,
            module: self.module.clone(),
        };
        let mut hook = |t: &TypeDesc| match t {
            TypeDesc::TypeOf(expr, _) => Some(TypeValue::Type(self.infer_expr(expr))),
            TypeDesc::Named(name, _) => self.lookup_type(name).map(TypeValue::Type),
            _ => None,
        };
        let mut ev = EvalCtx::new(init).with_hook(Some(&mut hook));
        let val = ev.eval_type(td);
        match &val {
            TypeValue::Closure(c) => {
                let params = c
                    .params
                    .iter()
                    .map(|p| match p {
                        Param::Typed(_, t) => t.to_string(),
                        _ => "any".to_owned(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = c
                    .body
                    .2
                    .as_ref()
                    .map(|r| {
                        r.tys
                            .iter()
                            .map(|t| t.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let arrow = if ret.is_empty() { "" } else { " -> " };
                Some(format!("type fn({params}){arrow}{ret}"))
            }
            _ => None,
        }
    }
}

impl TypeCheckerCtx<'_> {
    fn fn_type_ret(
        &mut self,
        body: &FuncBody,
        attrs: &Attrs,
        inferred: Option<&InferredReturns>,
    ) -> Type {
        let FuncBody(_, type_params, ret, _, _) = body;
        let names: Vec<&str> = type_params
            .iter()
            .map(|TypeParam((n, _), ..)| n.as_str())
            .collect();
        let declared: (Box<[Type]>, bool, Option<Box<str>>) = match ret {
            Some(r) => (
                if r.ensure.is_some() && r.tys.is_empty() {
                    // `: ensure C(T)` leaves the return type out on purpose, but
                    // it does not leave the function returning nothing: a
                    // postcondition is a claim about the value returned, and the
                    // value is a boolean. Reading the empty list as "no declared
                    // returns" would leave `return t ~= nil` unchecked and the
                    // function's own type claiming it returns nothing.
                    Box::from([Type::Bool])
                } else {
                    r.tys
                        .iter()
                        .map(|t| {
                            let normalized = normalize_generic_names(t, &names);
                            self.check_type_names(&normalized);
                            self.resolve_type(&normalized)
                        })
                        .collect()
                },
                r.var_arg,
                r.var_arg_bind.as_ref().map(|(name, span)| {
                    if !type_params
                        .iter()
                        .any(|TypeParam((n, _), .., mode, _)| n == name && *mode == ParamMode::Pack)
                    {
                        self.err(DukaSemanticError::UnknownType(name.clone().into()), *span);
                    }
                    name.clone().into_boxed_str()
                }),
            ),
            None => match inferred {
                // a return that does not always fill the last position is only
                // allowed to vary because the function declares `...`
                Some(r) => (r.types.clone(), r.var_arg && body.has_var_arg(), None),
                None => ([].into(), false, None),
            },
        };
        // written slots win, the protocol fills an absent declaration; `ret`
        // is read because `declared` may hold what the body inferred
        let written = matches!(&ret, Some(r) if r.ensure.is_some() || !r.tys.is_empty());
        let (returns, return_var_arg, return_var_arg_bind) = match attributes::get_returns(attrs) {
            Some(AttrReturns::Result) if !written => ([Type::Bool].into(), true, None),
            Some(AttrReturns::Result) => declared,
            Some(AttrReturns::Exit) => ([].into(), false, None),
            None => declared,
        };
        // The variables the signature declares travel with it, so a call site
        // can solve for them without knowing where the signature was written.
        let mut type_vars: Vec<TypeVar> = vec![];
        for TypeParam((name, _), bound, default, mode, prefer_literal) in type_params.iter() {
            type_vars.push(TypeVar {
                name: name.clone().into_boxed_str(),
                bound: bound
                    .as_ref()
                    .map(|b| Box::new(self.resolve_type(&normalize_generic_names(b, &names)))),
                default: default
                    .as_ref()
                    .map(|d| Box::new(self.resolve_type(&normalize_generic_names(d, &names)))),

                literal: *prefer_literal,
                pack: *mode == ParamMode::Pack,
            });
        }
        Type::Function(Some(FunctionType {
            params: body
                .named_params()
                .map(|(_, ty)| match ty {
                    Some(t) => {
                        let normalized = normalize_generic_names(t, &names);
                        self.check_type_names(&normalized);
                        self.resolve_type(&normalized)
                    }
                    None => Type::Any,
                })
                .collect(),
            var_arg: body.has_var_arg(),
            var_arg_bind: body.var_arg_bind().map(|(name, span)| {
                if !type_params
                    .iter()
                    .any(|TypeParam((n, _), .., mode, _)| n == name && *mode == ParamMode::Pack)
                {
                    self.err(DukaSemanticError::UnknownType(name.clone().into()), *span);
                }
                name.clone().into_boxed_str()
            }),
            returns,
            return_var_arg,
            return_var_arg_bind,
            type_vars: type_vars.into(),
        }))
    }
}

/// Rewrites every mention of a declared type parameter into `Type::Param`, the
/// match is exhaustive on purpose: a missing arm would silently turn the
/// parameter into `Any` instead of failing to compile
pub fn normalize_generic_names(tv: &TypeDesc, names: &[&str]) -> TypeDesc {
    match tv {
        TypeDesc::Pure(_) | TypeDesc::TypeOf(..) | TypeDesc::Splice(..) => tv.clone(),
        // a name in a type-level expression is a parameter too, and leaving it
        // alone would resolve it to `any` at every use
        TypeDesc::Expr(expr) => {
            TypeDesc::Expr(Box::new(normalize_generic_names_in_expr(expr, names)))
        }
        TypeDesc::Named(name, _) if names.contains(&name.as_ref()) => {
            TypeDesc::Pure(Type::Param(name.clone()))
        }
        TypeDesc::Named(..) => tv.clone(),
        TypeDesc::Generic(name, args, span) => TypeDesc::Generic(
            name.clone(),
            args.iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
            *span,
        ),
        // An application is an expression now, so the type parameters named inside it are
        // reached by the same normalisation as every other name in an expression --
        // `normalize_generic_names_in_expr` below. A second rule for the same
        // names would be a second thing to keep in step with the first.
        TypeDesc::Array(e) => TypeDesc::Array(
            e.as_deref()
                .map(|e| Box::new(normalize_generic_names(e, names))),
        ),
        TypeDesc::Table(k, v) => TypeDesc::Table(
            k.as_deref()
                .map(|k| Box::new(normalize_generic_names(k, names))),
            v.as_deref()
                .map(|v| Box::new(normalize_generic_names(v, names))),
        ),
        TypeDesc::Union(ts) => TypeDesc::Union(
            ts.iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
        ),
        TypeDesc::TypeTuple(ts, rest) => TypeDesc::TypeTuple(
            ts.iter()
                .map(|t| normalize_generic_names(t, names))
                .collect(),
            *rest,
        ),
        TypeDesc::TypeTable(ts) => TypeDesc::TypeTable(
            ts.iter()
                .map(|f| {
                    crate::parser::ast::TypeField::new(
                        f.key.clone(),
                        f.span,
                        normalize_generic_names(&f.ty, names),
                    )
                })
                .collect(),
        ),
        TypeDesc::Function(ft) => TypeDesc::Function(ft.as_ref().map(|ft| {
            TypeFnValue {
                params: ft
                    .params
                    .iter()
                    .map(|t| normalize_generic_names(t, names))
                    .collect(),
                var_arg: ft.var_arg,
                returns: ft
                    .returns
                    .iter()
                    .map(|t| normalize_generic_names(t, names))
                    .collect(),
                return_var_arg: ft.return_var_arg,
                return_var_arg_bind: ft.return_var_arg_bind.clone(),
            }
        })),
        TypeDesc::FnLit(body) => {
            let mut cloned = body.as_ref().clone();
            let FuncBody(params, _, ret, _, _) = &mut cloned;
            for p in params.iter_mut() {
                if let Param::Typed(_, t) = p {
                    *t = normalize_generic_names(t, names);
                }
            }
            if let Some(r) = ret.as_mut() {
                for t in r.tys.iter_mut() {
                    *t = normalize_generic_names(t, names);
                }
            }
            TypeDesc::FnLit(Box::new(cloned))
        }
        TypeDesc::NonNil(inner) => {
            TypeDesc::NonNil(Box::new(normalize_generic_names(inner, names)))
        }
        TypeDesc::Nilable(inner) => {
            TypeDesc::Nilable(Box::new(normalize_generic_names(inner, names)))
        }
        TypeDesc::Rec(inner) => TypeDesc::Rec(Box::new(normalize_generic_names(inner, names))),
    }
}

/// The same rewrite inside a type-level expression. A bare name can only appear
/// as a `TypeLit`, so that is the only place a parameter can hide; the walk is
/// over the whole tree because an expression can nest records in calls.
fn normalize_generic_names_in_expr(expr: &Expr, names: &[&str]) -> Expr {
    let mut cloned = expr.clone();
    let mut rewrite = NormalizeGenericNames { names };
    rewrite.walk(&mut cloned);
    cloned
}

struct NormalizeGenericNames<'a> {
    names: &'a [&'a str],
}

impl NormalizeGenericNames<'_> {
    /// Rewrites every `TypeLit` in the expression tree. `Expr` has no
    /// `VisitorMut` derive, so the walk is spelled out over the node kinds a
    /// type-level expression may hold; a kind the gate refuses is not here.
    fn walk(&mut self, expr: &mut Expr) {
        match &mut expr.0 {
            NodeKind::TypeLit(td) => *td = normalize_generic_names(td, self.names),
            NodeKind::Call(callee, args) => {
                self.walk(callee);
                for arg in args.iter_mut() {
                    self.walk(arg);
                }
            }
            NodeKind::Unary(inner, _) => self.walk(inner),
            NodeKind::Binary(l, r, _) => {
                self.walk(l);
                self.walk(r);
            }
            _ => (),
        }
    }
}

impl<'a> TypeCheckerCtx<'a> {
    /// Records which calls are operands of a `|>`.
    ///
    /// The walk is post-order, so a call is reached before the pipe that holds
    /// it is; the set is built up front so the statement reading can tell them
    /// apart.
    fn mark_pipe_operands(&mut self, chunk: &DukaChunk) {
        struct Mark<'b>(&'b mut std::collections::HashSet<Span>);
        impl Visitor for Mark<'_> {
            fn visit_node(&mut self, node: &Expr) {
                match &node.0 {
                    NodeKind::Binary(_, r, BinOp::Pipeline(_)) => {
                        if matches!(r.0, NodeKind::Call(..)) {
                            self.0.insert(r.1);
                        }
                    }
                    NodeKind::Binary(l, _, BinOp::PipelineL) => {
                        if matches!(l.0, NodeKind::Call(..)) {
                            self.0.insert(l.1);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut mark = Mark(&mut self.pipe_operands);
        chunk.visit(&mut mark);
    }

    fn infer_pipe_fold(&mut self, stem: &Node, head: &Node, op: &BinOp) -> Type {
        let (path, span, written_args) = match (&stem.0, stem.1) {
            (NodeKind::Call(callee, args), span) => match &**callee {
                Node(NodeKind::Access(path), _) => (path.as_ref().clone(), span, args.iter()),
                other => (
                    Path::Expr(Box::new(Node(other.0.clone(), span))),
                    span,
                    args.iter(),
                ),
            },
            (NodeKind::Access(path), span) => (path.as_ref().clone(), span, [].iter()),
            _ => return Type::Any,
        };
        let mut folded: Vec<Expr> = Vec::with_capacity(written_args.len() + 1);
        if matches!(op, BinOp::Pipeline(_)) {
            folded.push(head.clone());
        }
        folded.extend(written_args.cloned());
        if !matches!(op, BinOp::Pipeline(_)) {
            folded.push(head.clone());
        }
        match self.infer_call(&path, span, &folded) {
            Some((a, true)) if a.is_empty() => Type::Any,
            Some((v, _)) => v.into_iter().next().unwrap_or(Type::Nil),
            None => Type::Any,
        }
    }
}

/// The type a plain alias declaration stands for, when it can be read without
/// evaluating a type function.
///
/// This is only used to name a type that is already in hand, so a declaration
/// that computes is left alone: the expansion it would produce is not something
/// this can know before the evaluator runs.
fn alias_type(desc: &TypeDesc) -> Option<Type> {
    match desc {
        TypeDesc::Pure(ty) => Some(ty.clone()),
        TypeDesc::Union(parts) => {
            let mut acc = Type::Never;
            for part in parts.iter() {
                acc = acc | alias_type(part)?;
            }
            Some(acc)
        }
        TypeDesc::Array(inner) => {
            let inner = match inner.as_deref() {
                Some(inner) => Some(Box::new(alias_type(inner)?)),
                None => None,
            };
            Some(Type::Array(inner))
        }
        TypeDesc::Table(key, value) => {
            let key = match key.as_deref() {
                Some(key) => Some(Box::new(alias_type(key)?)),
                None => None,
            };
            let value = match value.as_deref() {
                Some(value) => Some(Box::new(alias_type(value)?)),
                None => None,
            };
            Some(Type::Table(key, value, None))
        }
        TypeDesc::TypeTuple(parts, rest) => {
            let parts = parts.iter().map(alias_type).collect::<Option<Vec<_>>>()?;
            Some(Type::TypeTuple(parts, *rest))
        }
        TypeDesc::Nilable(inner) => Some(alias_type(inner)? | Type::Nil),
        TypeDesc::NonNil(inner) => alias_type(inner),
        _ => None,
    }
}

/// What the variables of a generic `for` hold, given what it iterates.
///
/// An array yields its elements, a table yields its values when one name is
/// written and its keys and values when two are. The runtime decides that from
/// how many names there are, so the reading does the same.
fn for_var_types(ty: &Type, count: usize) -> Option<Vec<Type>> {
    if count == 0 {
        return Some(vec![]);
    }
    match ty {
        Type::Array(inner) => {
            let mut vars = vec![inner.as_deref().cloned().unwrap_or(Type::Any)];
            vars.resize(count, Type::Nil);
            Some(vars)
        }
        Type::Table(key, value, _) => {
            let key = key.as_deref().cloned().unwrap_or(Type::Any);
            let value = value.as_deref().cloned().unwrap_or(Type::Any);
            let mut vars = if count == 1 {
                vec![value]
            } else {
                vec![key, value]
            };
            vars.resize(count, Type::Nil);
            Some(vars)
        }
        Type::Union(types) => {
            let mut combined: Option<Vec<Type>> = None;
            for ty in types.iter() {
                let Some(vars) = for_var_types(ty, count) else {
                    continue;
                };
                match &mut combined {
                    Some(acc) => {
                        for (slot, var) in acc.iter_mut().zip(vars) {
                            *slot = slot.clone() | var;
                        }
                    }
                    None => combined = Some(vars),
                }
            }
            combined
        }
        Type::Rec(inner) => for_var_types(inner, count),
        _ => None,
    }
}

impl<'a> Visitor for TypeCheckerCtx<'a> {
    /// Puts what a condition proves at the head of the block it guards.
    ///
    /// The block is walked before the condition is -- the condition is the field
    /// after it -- so the facts have to be read here, while the block's frame is
    /// the one on top. The reading costs a second inference of the condition,
    /// whose diagnostics are dropped: the walk reaches the condition itself
    /// afterwards and reports the same mistakes there.
    fn visit_if_clause_block(&mut self, clause: &IfClause, enter: bool) {
        if !enter {
            return;
        }
        let cond = &clause.1;
        let saved = std::mem::take(&mut self.errors);
        self.infer_condition_calls(cond);
        self.errors = saved;
        // The clauses above this one were false by the time it is read, so
        // what failing them promised goes in first. The list on top belongs to
        // this clause's own `if` when it holds this clause's condition; an
        // inner `if` reads its first clause with the list of the `if` around
        // it on top, and nothing there was false before that clause.
        if let Some(conds) = self.clause_conds.last()
            && let Some(pos) = conds.iter().position(|c| c.1 == cond.1)
        {
            let mut negated = vec![];
            for earlier in &conds[..pos] {
                negated.extend(self.condition_facts(earlier, false));
            }
            for c in negated {
                self.apply(&c);
            }
        }
        let guarantees = self.condition_facts(cond, true);
        for c in guarantees {
            self.apply(&c);
        }
    }

    /// Keeps the conditions of an `if` while the clauses after the first are
    /// walked, so a later clause can read the earlier ones as having failed.
    ///
    /// The first clause is a field of its own and the hook fires around the
    /// rest, so a first clause enters with the list of the `if` around it --
    /// or none -- on top, which is exactly what it needs: nothing above it has
    /// been read yet.
    fn visit_if_clauses_block(&mut self, if_: &If, enter: bool) {
        if enter {
            self.clause_conds.push(
                std::iter::once(&if_.0)
                    .chain(if_.1.iter())
                    .map(|clause| clause.1.clone())
                    .collect(),
            );
        } else {
            self.clause_conds.pop();
        }
    }

    /// Puts what failing the conditions proves at the head of the `else`.
    ///
    /// The conditions were all walked before the `else` is reached -- a
    /// condition is the field after its block, and the `else` is the last
    /// field of the `if` -- so their promises are already waiting.
    fn visit_if_else_block(&mut self, if_: &If, enter: bool) {
        if !enter || if_.2.is_none() {
            return;
        }
        for clause in std::iter::once(&if_.0).chain(if_.1.iter()) {
            let guarantees = self.condition_facts(&clause.1, false);
            for c in guarantees {
                self.apply(&c);
            }
        }
    }

    /// Puts what a loop's condition proves at the head of its body.
    ///
    /// A loop reads its condition before its block, so by the time the body is
    /// entered the promise is already waiting.
    fn visit_loop_stmt_block(&mut self, kind: &NodeKind, enter: bool) {
        if !enter {
            return;
        }
        if let NodeKind::ForGeneric(paths, exprs, ..) = kind {
            self.declare_for_vars(paths, exprs);
            return;
        }
        let cond = match kind {
            NodeKind::While(cond, ..) => &**cond,
            NodeKind::ForNumeric(_, _, cond, ..) => &**cond,
            _ => return,
        };
        let guarantees = self.condition_facts(cond, true);
        for c in guarantees {
            self.apply(&c);
        }
    }

    fn visit_node(&mut self, stmt: &crate::parser::ast::Stmt) {
        // A statement-only node was never inferred. The arms below are the whole
        // of what the checker has ever said about it, and `infer_expr` would now
        // be handed forms it has never seen.
        if !stmt.0.is_stmt_only() {
            let outer = std::mem::replace(&mut self.final_args, false);
            let inferred = self.infer_expr(stmt);
            self.final_args = outer;
            self.expr_types.insert(stmt.1, inferred);
            if self.collect_mode
                && let NodeKind::Function(body) = &stmt.0
                && body.2.is_none()
                && let Some(returns) = self.finished_returns.pop()
                && !returns.types.is_empty()
            {
                self.collected_closures.insert(body_span(body), returns);
            }
        }
        match &stmt.0 {
            NodeKind::Define(names, exprs, _, _) => {
                let (tys, _var_arg) = self.infer_expr_list(exprs);
                for (idx, (((name, span), attrs, ty), _)) in names.iter().enumerate() {
                    // A declaration says what exists, so there is no value here to
                    // be compatible with and nothing to infer one from: the
                    // annotation is the whole of the type, and no annotation means
                    // the language was told nothing and has nothing to say. `nil`
                    // would be a reading of a value that was never written.
                    if attributes::has_declare(attrs) {
                        if exprs.get(idx).is_some() {
                            self.err(
                                DukaSemanticError::DeclaredIsNotBuilt(name.as_str().into()),
                                exprs.get(idx).map(|e| e.1).unwrap_or(*span),
                            );
                        }
                        let declared = match ty {
                            Some(t) => {
                                self.check_type_names(t);
                                self.resolve_type(t)
                            }
                            None => Type::Any,
                        };
                        self.declare(name, *span, declared);
                        continue;
                    }
                    let actual = if let Some(ty) = tys.get(idx) {
                        let cv = match exprs.get(idx) {
                            Some(Node(NodeKind::Literal(cv), _)) => Some(cv.clone()),
                            _ => None,
                        };
                        (ty.clone(), cv)
                    } else {
                        (Type::Nil, None)
                    };
                    let declared = ty.as_ref().map(|t| {
                        self.check_type_names(t);
                        self.resolve_type(t)
                    });
                    if let Some(declared) = &declared
                        && !declared.accepts_value(&actual.0, actual.1.as_ref())
                    {
                        // The value is wrong and the type it was asked to be is
                        // where the reader has to go next, so the annotation is
                        // pointed at rather than left for them to find.
                        let asked_for =
                            ty.as_ref()
                                .and_then(|desc| desc.span())
                                .map(|where_written| {
                                    (
                                        "the type it was asked to be is written here".into(),
                                        where_written,
                                    )
                                });
                        self.err_opt_related(
                            DukaSemanticError::TypeMismatchEqual(
                                self.display_type(declared),
                                self.display_type(&actual.0),
                            ),
                            exprs.get(idx).map(|e| e.1).unwrap_or(*span),
                            asked_for,
                        );
                    }
                    // The adapter lowers a class into a local carrying the class's name, which is
                    // how the class exists at runtime: a Lua class is a table with
                    // an `__index`. So a local written under a class's name is
                    // that class rather than the table it was lowered into, and it
                    // is kept as the class so a member read or a method call off
                    // it has the class to answer with. An annotation still wins:
                    // naming a type says what this local is, whatever it shadows.
                    let lowered = class_named(self, name.as_str());
                    match declared.or(lowered) {
                        Some(ty) => self.declare(name, *span, ty),
                        None => self.declare_inferred(
                            name,
                            *span,
                            actual.0,
                            nil_literal_or_none(exprs.get(idx)),
                        ),
                    }
                }
            }
            NodeKind::Destructing(names, expr, _) => {
                // A destructuring reads its pattern apart against one value, so a
                // name takes that value indexed by where the name sits. It cannot
                // be left to the expanded shape: that reads the pattern into
                // `tmp.a`, and `tmp` is a table of no particular type, so the
                // field comes back `any` and the name is told nothing.
                let value = self.infer_expr(expr);
                self.declare_destructured(names, &value);
            }
            NodeKind::Assign(targets, exprs) => {
                let (tys, _var_arg) = self.infer_expr_list(exprs);
                for (idx, target) in targets.iter().enumerate() {
                    let actual = if let Some(ty) = tys.get(idx) {
                        let cv = match exprs.get(idx) {
                            Some(Node(NodeKind::Literal(cv), _)) => Some(cv.clone()),
                            _ => None,
                        };
                        (ty.clone(), cv)
                    } else {
                        (Type::Nil, None)
                    };
                    if let Path::Base((name, span)) = target
                        && self.lookup_type(name).is_none()
                    {
                        self.declare_inferred(
                            name,
                            *span,
                            actual.0,
                            nil_literal_or_none(exprs.get(idx)),
                        );
                        continue;
                    }
                    // writing to a name is what ends every fact about what it
                    // held, and it ends them before the value is checked: the
                    // value answers the declaration, not the narrowing
                    if let Path::Base((name, _)) = target {
                        self.forget(name);
                    }
                    if let Some((owner, key, key_span)) = assign_target(target) {
                        self.assign_into(
                            owner,
                            key,
                            &actual.0,
                            exprs.get(idx).map(|e| e.1).unwrap_or(key_span),
                        );
                        continue;
                    }

                    if let Path::Base((name, sp)) = target
                        && let Some(declared) = self.lookup_type(name)
                        && !declared.accepts_value(&actual.0, actual.1.as_ref())
                    {
                        self.err(
                            DukaSemanticError::TypeMismatchEqual(
                                self.display_type(&declared),
                                self.display_type(&actual.0),
                            ),
                            exprs.get(idx).map(|e| e.1).unwrap_or(*sp),
                        );
                    }
                }
            }
            NodeKind::Return(items, ..) => {
                if self.collect_mode {
                    let (collected, _) = self.infer_expr_list(items);
                    if let Some(buf) = self.ret_collect.last_mut() {
                        buf.merge(&collected);
                    }
                }
                let ret = self.ret_stack.last().cloned().flatten();

                // if ret.is_none() -> void

                // the values are inferred either way, an unknown return type
                // only means there is nothing to check them against
                let (tys, var_arg_get) = self.infer_expr_list(items);
                if let Some((fixeds, var_arg)) = ret {
                    if tys.len() < fixeds.len() && !var_arg_get {
                        self.err(
                            DukaSemanticError::TypeMismatchReturn(
                                fixeds
                                    .iter()
                                    .map(|f| f.to_string())
                                    .chain(var_arg.then_some("...".to_owned()))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                format!("{} values", tys.len()),
                            ),
                            stmt.1,
                        );
                    }
                    for (idx, ty) in tys.into_iter().enumerate() {
                        let cv = match items.get(idx) {
                            Some(Node(NodeKind::Literal(cv), _)) => Some(cv.clone()),
                            _ => None,
                        };
                        let Some(expected) = fixeds.get(idx).cloned() else {
                            break; // whether var_arg or not, drop them
                        };
                        // a generic return type is only known at the call site
                        if mentions_param(&expected) {
                            continue;
                        }
                        if !expected.accepts_value(&ty, cv.as_ref()) {
                            self.err(
                                DukaSemanticError::TypeMismatchReturn(
                                    self.display_type(&expected),
                                    self.display_type(&ty),
                                ),
                                items.get(idx).map(|e| e.1).unwrap_or(stmt.1),
                            );
                        }
                    }
                }
            }
            NodeKind::FunctionDecl(path, attrs, body, _) => {
                if attributes::has_declare(attrs)
                    && let Some(r) = body.2.as_ref()
                {
                    let declared = r.span;
                    self.errors.retain(|e| {
                        !matches!(
                            e.kind,
                            DukaErrorKind::Semantic(DukaSemanticError::NotAllReturn)
                        ) || e.span != declared
                    });
                }
                if let Path::Base((name, span)) = path {
                    for TypeParam((ty_par, par_span), ..) in body.1.iter() {
                        self.declare(
                            ty_par,
                            *par_span,
                            Type::Param(ty_par.clone().into_boxed_str()),
                        );
                    }
                    self.run_where(
                        &body.3,
                        &body
                            .1
                            .iter()
                            .map(|TypeParam((n, _), ..)| n.clone().into_boxed_str())
                            .collect::<Vec<_>>(),
                    );
                    // The frame this wrote into is not popped here. `where` introduces names of
                    // its own as well, and those belong to the function, so the
                    // scope that holds both is the body's, entered below.
                    let ensure = Self::ensure_of(body);
                    if !body.1.is_empty() || ensure.is_some() {
                        self.generic_fns.insert(
                            name.clone().into_boxed_str(),
                            GenericDecl {
                                params: body.1.clone(),
                                constraints: body.3.clone(),
                                ensure,
                            },
                        );
                    }
                    let ty = match &body.2 {
                        Some(_) => self.fn_type_ret(body, attrs, None),
                        None => {
                            let inferred = self.inferred_returns.get(name.as_str()).cloned();
                            match inferred {
                                Some(r) => self.fn_type_ret(body, attrs, Some(&r)),
                                // `fn_type` is `fn_type_ret` with no attributes,
                                // which drops what the attributes say at the very
                                // moment there is nothing inferred to say instead.
                                // A `@returns(result)` on a bodyless function is
                                // exactly that case, and it used to read as a
                                // function that returns nothing.
                                None => self.fn_type_ret(body, attrs, None),
                            }
                        }
                    };
                    // A declaration is not run, so a body under one is not
                    // inferred into it either: the signature is what was written
                    // down and the rest is a declaration that was not what it
                    // looked like.
                    if attributes::has_declare(attrs) && !body.4.0.is_empty() {
                        self.err(
                            DukaSemanticError::DeclaredIsNotBuilt(name.as_str().into()),
                            body_span(body),
                        );
                    }
                    self.declare(name, *span, ty);
                }
                if self.collect_mode
                    && let Some(returns) = self.finished_returns.pop()
                    && let Path::Base((name, _)) = path
                    && body.2.is_none()
                    && !returns.types.is_empty()
                {
                    self.collected_returns
                        .insert(name.clone().into_boxed_str(), returns);
                }
            }
            NodeKind::Expr(expr) => {
                self.infer_expr(expr);
            }
            NodeKind::TypeAlias((_, span), ty) => {
                let resolved = self.resolve_type(ty);
                let display = if resolved == Type::Any {
                    self.resolve_display(ty)
                        .unwrap_or_else(|| resolved.to_string())
                } else {
                    resolved.to_string()
                };
                if !self.collect_mode {
                    let id = self.viewer.table().symbol_at_span(*span).map(|s| s.id);
                    self.backfills
                        .push((id, *span, display.into_boxed_str(), Some(resolved)));
                }
            }
            NodeKind::TypeFunction((_, span), body) => {
                if !self.collect_mode {
                    let id = self.viewer.table().symbol_at_span(*span).map(|s| s.id);
                    self.backfills.push((
                        id,
                        *span,
                        format!(
                            "function({})",
                            std::iter::repeat_n("type", body.0.len())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                        .into_boxed_str(),
                        None,
                    ));
                }
            }
            NodeKind::Call(callee, args) => {
                // a call inside a pipe is checked as the fold will read it,
                // not as it is written; the expression reading above already
                // inferred it
                if !self.pipe_operands.contains(&stmt.1)
                    && let Node(NodeKind::Access(path), span) = &**callee
                {
                    self.infer_call(path, *span, args);
                }
            }
            NodeKind::Export(inner) => {
                // statements are visited post order, so the exported symbols
                // already carry their types here
                for (name, _) in exported_names(&inner.0) {
                    // A class is declared by the scope pass rather than here, so
                    // it has no type in the frames to read back; it is built from
                    // the table instead. Every class built here is stamped with
                    // the module it was declared in, which is what lets an importer
                    // read its fields and hand its instances back.
                    let ty = self
                        .lookup_type(&name)
                        .or_else(|| self.object_id_of(&name).map(|id| self.object_of(id)));
                    if let Some(ty) = ty {
                        // read off the symbol rather than off the statement, so
                        // what the importer is told is the same reading the
                        // declaration itself was given
                        if let Some(attribute) = self
                            .viewer
                            .lookup_anywhere(&name)
                            .map(|sym| sym.attributes.clone())
                        {
                            self.export_attributes.insert(name.clone(), attribute);
                        }
                        self.export_members.insert(name, ty);
                    }
                }
            }

            _ => {}
        }

        match &stmt.0 {
            NodeKind::Unary(e, op) => {
                if let UnOp::BitNot = op
                    && let Node(NodeKind::Literal(ConstValue::Float(_)), span) = &**e
                {
                    self.err(
                        DukaSemanticError::TypeMismatchEqual(
                            Type::Int.to_string(),
                            Type::Float.to_string(),
                        ),
                        *span,
                    );
                }
            }
            NodeKind::Binary(a, b, op) => {
                if matches!(
                    op,
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::ShiftL | BinOp::ShiftR
                ) {
                    let both_literal =
                        matches!(a.0, NodeKind::Literal(_)) && matches!(b.0, NodeKind::Literal(_));
                    if both_literal {
                        for operand in [a.as_ref(), b.as_ref()] {
                            if matches!(operand.0, NodeKind::Literal(ConstValue::Float(_))) {
                                self.err(
                                    DukaSemanticError::TypeMismatchEqual(
                                        Type::Int.to_string(),
                                        Type::Float.to_string(),
                                    ),
                                    operand.1,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn before(&mut self) {}

    fn after(&mut self) {}

    fn visit_func_block(&mut self, block: &FuncBody, enter: bool) {
        if enter {
            for TypeParam((name, span), bound, ..) in block.1.iter() {
                self.declare(name, *span, Type::Param(name.clone().into_boxed_str()));
                // The bound goes into the parameter's own frame, under a name nothing can
                // collide with. The frame is what scopes it.
                if let Some(bound) = bound {
                    let bound = self.resolve_type(bound);
                    self.declare(&bound_key(name), *span, bound);
                }
            }

            for clause in &block.3 {
                if let WhereClause::Bind(names, value, _) = clause {
                    let ty = self.resolve_type(value);
                    for bind in destructuring::bind(names, &ty) {
                        if let destructuring::Bind::Named(name, found) = bind {
                            self.declare(&name.0, name.1, found.unwrap_or_else(|| ty.clone()));
                        }
                    }
                }
            }
            let ret = block.2.as_ref().and_then(|r| {
                let tys: Vec<Type> = if r.ensure.is_some() && r.tys.is_empty() {
                    // The same `bool` the function's own type is built with.
                    // A postcondition is a claim about the value returned, and a
                    // claim about a value nobody returns is not a claim.
                    vec![Type::Bool]
                } else {
                    r.tys.iter().map(|t| self.resolve_type(t)).collect()
                };

                if (tys.is_empty() || matches!(tys.as_slice(), [Type::Never])) && !r.var_arg {
                    None
                } else {
                    if !block.4.always_returns() {
                        self.err(DukaSemanticError::NotAllReturn, r.span);
                    }
                    Some((tys, r.var_arg))
                }
            });

            self.ret_stack.push(ret);
            self.declare_params(block);
            if let Some(id) = self.method_owner.get(&objects::func_body_span(block)).copied() {
                let ty = self.object_of(id);
                self.declare("self", Span::EMPTY, ty);
            }
            // a promise made about a name of the caller's is not about any name
            // of this body's, so nothing pending goes in with it
            self.pending.clear();
            if self.collect_mode {
                self.ret_collect.push(InferredReturns::default());
            }
        } else {
            if self.collect_mode {
                let collected = self.ret_collect.pop().unwrap_or_default();
                self.finished_returns.push(collected);
            }
            self.ret_stack.pop();
        }
    }

    fn visit_block(&mut self, enter: bool) {
        if enter {
            self.types.push(HashMap::new());
            self.facts.push(HashMap::new());
            self.viewer.enter();
        } else {
            self.viewer.exit();
            self.facts.pop();
            self.types.pop();
        }
    }

    /// A `do` is worth what its `break` carries, and what the `break` reads is
    /// declared inside the block -- so the reading belongs at the exit, where the
    /// block has been walked, rather than at the expression reading that got
    /// here first. A function collects its returns the same way.
    fn visit_do_stmt_block(&mut self, kind: &NodeKind, enter: bool) {
        if enter {
            return;
        }
        let NodeKind::Do(block, _, label) = kind else {
            return;
        };
        if self.collect_mode {
            let tys = break_types_of(self, block, label.as_ref().map(|n| n.0.as_str()));
            self.collected_do_values.insert(do_span(block), tys);
        }
    }
}

impl TypeCheckerCtx<'_> {
    #[inline(always)]
    fn infer_expr(&mut self, Node(kind, _): &Expr) -> Type {
        self.infer_expr_kind(kind)
    }

    fn infer_expr_list(&mut self, list: &[Expr]) -> Multi {
        let mut res = vec![];
        let mut va = false;
        if let Some((last, rest)) = list.split_last() {
            for e in rest {
                res.push(self.infer_expr_kind(&e.0));
            }
            let (mut ts, var_arg) = self.infer_expr_kind_multi(&last.0);
            res.append(&mut ts);
            va = var_arg;
        }
        (res, va)
    }

    fn infer_expr_kind_multi(&mut self, kind: &NodeKind) -> Multi {
        match kind {
            NodeKind::VarArg => (vec![], true),
            NodeKind::Call(callee, args) => {
                if let Node(NodeKind::Access(path), span) = &**callee
                    && let Some(a) = self.infer_call(path, *span, args)
                {
                    return a;
                }
                // a callee that is itself a call (`f()()`) is not a path, so it
                // is asked what it evaluates to rather than what it names
                if !matches!(callee.0, NodeKind::Access(_))
                    && let Some(a) = self.infer_call(&Path::Expr(callee.clone()), callee.1, args)
                {
                    return a;
                }
                (vec![Type::Any], true)
            }
            // a `do` is worth several values when its `break` carries them, so
            // it answers as a tuple rather than as one type
            NodeKind::Do(block, _, label) => (
                do_value(self, block, label.as_ref().map(|n| n.0.as_str())),
                false,
            ),
            ek => (vec![self.infer_expr_kind(ek)], false),
        }
    }

    fn infer_expr_kind(&mut self, kind: &NodeKind) -> Type {
        match kind {
            NodeKind::Literal(lit) => lit.type_of(),
            // a `do` hands back whatever its `break` carried, and one that
            // carries nothing is worth a nil
            NodeKind::Do(block, _, label) => {
                let mut tys = do_value(self, block, label.as_ref().map(|n| n.0.as_str()));
                match tys.len() {
                    0 => Type::Nil,
                    1 => tys.swap_remove(0),
                    _ => Type::TypeTuple(tys, false),
                }
            }
            NodeKind::Table(fields) => {
                if fields.is_empty() {
                    return Type::Table(None, None, None);
                }
                if !fields.iter().all(|f| matches!(f, Field::Value(_))) {
                    let mut vec = vec![];
                    for f in fields {
                        if let Field::NameValue(n, v) = f {
                            let e = &self.infer_expr_kind(&v.0);
                            vec.push((
                                ConstValue::String(n.0.as_bytes().to_vec().into_boxed_slice()),
                                Box::new(e.clone()),
                            ))
                        } else {
                            return Type::Table(None, None, None);
                        }
                    }
                    return Type::TypeTable(vec, None);
                }
                let last = fields.len().saturating_sub(1);
                let mut elems = vec![];
                for (idx, f) in fields.iter().enumerate() {
                    let Field::Value(e) = f else { continue };
                    if idx == last {
                        let (mut ts, _) = self.infer_expr_kind_multi(&e.0);
                        elems.append(&mut ts);
                    } else {
                        elems.push(self.infer_expr(e));
                    }
                }
                Type::TypeTuple(elems, false)
            }
            NodeKind::Array(items) => {
                let (elems, rest) = self.infer_expr_list(items);
                Type::TypeTuple(elems, rest)
            }
            NodeKind::Function(body) => {
                // a closure has no name to key its inferred returns on, so they
                // are held by where it was written
                let inferred = self.inferred_closures.get(&body_span(body)).cloned();
                let ft @ Type::Function(Some(_)) =
                    self.fn_type_ret(body, &Attrs::default(), inferred.as_ref())
                else {
                    return Type::Any;
                };
                ft
            }
            NodeKind::Access(path) => self.infer_access(path),
            NodeKind::Call(callee, args) => match &**callee {
                Node(NodeKind::Access(path), span) => match self.infer_call(path, *span, args) {
                    Some((a, true)) if a.is_empty() => Type::Any,
                    Some((v, _)) => v.into_iter().next().unwrap_or(Type::Nil),
                    None => Type::Never,
                },
                // a callee that is itself a call (`f()()`) is not a path, so it
                // is asked what it evaluates to rather than what it names
                _ => match self.infer_call(&Path::Expr(callee.clone()), callee.1, args) {
                    Some((a, true)) if a.is_empty() => Type::Any,
                    Some((v, _)) => v.into_iter().next().unwrap_or(Type::Nil),
                    None => Type::Any,
                },
            },
            NodeKind::Unary(e, op) => {
                let ty = self.infer_expr(e);
                match op {
                    UnOp::Minus => match &ty {
                        Type::Int | Type::Float => ty,
                        other => self
                            .meta_result(other, MetaMethod::Unm.name())
                            .unwrap_or(Type::Any),
                    },
                    UnOp::Length => match &ty {
                        Type::String => Type::Int,
                        other => self
                            .meta_result(other, MetaMethod::Len.name())
                            .unwrap_or(Type::Any),
                    },
                    UnOp::Not => match ty {
                        Type::Bool => Type::Bool,
                        _ => Type::Any,
                    },
                    UnOp::BitNot => match &ty {
                        Type::Int | Type::Bool => ty,
                        other => self
                            .meta_result(other, MetaMethod::BNot.name())
                            .unwrap_or(Type::Any),
                    },
                }
            }
            NodeKind::Binary(a, b, op) => match op {
                BinOp::Concat => {
                    let lt = self.infer_expr(a);
                    let rt = self.infer_expr(b);
                    self.meta_binary(&lt, &rt, MetaMethod::Concat.name())
                        .unwrap_or(Type::String)
                }
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::ShiftL | BinOp::ShiftR => {
                    let lt = self.infer_expr(a);
                    let rt = self.infer_expr(b);
                    match (&lt, &rt) {
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Bool, Type::Bool) => Type::Bool,
                        _ => self
                            .meta_binary(&lt, &rt, binop_metamethod(op).unwrap_or(""))
                            .unwrap_or(Type::Any),
                    }
                }
                BinOp::Add
                | BinOp::Sub
                | BinOp::Multiply
                | BinOp::Divide
                | BinOp::IDivide
                | BinOp::Mod
                | BinOp::Pow => {
                    let lt = self.infer_expr(a);
                    let rt = self.infer_expr(b);
                    match (&lt, &rt) {
                        (Type::Float, Type::Float) => Type::Float,
                        (Type::Int, Type::Int) => Type::Int,
                        (Type::Float, Type::Int) | (Type::Int, Type::Float) => Type::Float,
                        _ => self
                            .meta_binary(&lt, &rt, binop_metamethod(op).unwrap_or(""))
                            .unwrap_or(Type::Any),
                    }
                }
                BinOp::Pipeline(_) | BinOp::PipelineL => match op {
                    BinOp::Pipeline(_) => self.infer_pipe_fold(b, a, op),
                    _ => self.infer_pipe_fold(a, b, op),
                },
                _ => Type::Bool,
            },
            _ => Type::Any,
        }
    }

    #[inline]
    fn object_id_of(&self, name: &str) -> Option<ObjectId> {
        self.viewer
            .lookup_anywhere(name)
            .and_then(|sym| match &sym.symbol_type {
                SymbolType::ObjectClass(id) => Some(*id),
                _ => None,
            })
    }

    /// The class a type names, wherever it was declared.
    ///
    /// A class that crossed a `require` carries the module it lives in, and its
    /// id is an index into *that* module's table rather than into this file's.
    /// Both tables are numbered from zero, so answering out of the wrong one
    /// would be a plausible-looking wrong class rather than a failure -- which is
    /// why the answer is looked up by where the class was declared and not by
    /// where the type was read.
    #[inline]
    fn object_class(&self, ty: &Type) -> Option<&ObjectType> {
        let Type::Object { id, module, .. } = ty else {
            return None;
        };
        match module {
            None => self.objects.get(*id),
            Some(key) => self
                .modules
                .get(key.as_ref())
                .and_then(|m| m.analysis.objects.get(*id)),
        }
    }

    /// A class id read in the table `recv`'s class was declared in.
    fn class_by_id(&self, recv: &Type, id: ObjectId) -> Option<&ObjectType> {
        let Type::Object { module, .. } = recv else {
            return None;
        };
        match module {
            None => self.objects.get(id),
            Some(key) => self
                .modules
                .get(key.as_ref())
                .and_then(|m| m.analysis.objects.get(id)),
        }
    }

    /// A member of the class, its bases included.
    fn class_member(&self, recv: &Type, name: &str) -> Option<TypeDesc> {
        let mut class = self.object_class(recv);
        while let Some(c) = class {
            if let Some(m) = c.members.iter().find(|m| m.name.as_ref() == name) {
                return Some(m.ty.clone());
            }
            class = c.base.and_then(|base| self.class_by_id(recv, base));
        }
        None
    }

    /// The object a type names, or the one a union offers.
    ///
    /// An annotation is nilable by default, so the type of an instance is
    /// `Point?` and the class is inside the union rather than being the type
    /// itself. A class that cannot be resolved answers nothing: a type naming a
    /// module that is not loaded is not a class here, and saying so is better
    /// than a method lookup that quietly finds nothing.
    fn object_of_type(&self, ty: &Type) -> Option<Type> {
        let found = match ty {
            Type::Object { .. } => Some(ty.clone()),
            Type::Union(ts) => ts.iter().find_map(|t| self.object_of_type(t)),
            _ => None,
        }?;
        self.object_class(&found).is_some().then_some(found)
    }

    #[inline]
    fn receiver_object(&mut self, path: &Path) -> Option<Type> {
        match path {
            Path::Base((name, _)) => {
                if let Some(obj) = self
                    .lookup_type(name)
                    .and_then(|ty| self.object_of_type(&ty))
                {
                    return Some(obj);
                }
                self.object_id_of(name).map(|id| self.object_of(id))
            }
            _ => {
                // resolve the chain by type, `a.b:c()` looks `c` up on the type
                // of `a.b` instead of on the root of the chain
                let ty = self.type_of_path(path);
                self.object_of_type(&ty)
            }
        }
    }

    #[inline]
    fn object_of(&self, id: ObjectId) -> Type {
        let obj = &self.objects[id];
        Type::Object {
            id,
            name: obj.name.clone(),
            base: obj.base,
            args: [].into(),
            module: self.module.clone(),
            mt: None,
        }
    }

    #[inline]
    fn return_type_of(&self, sig: &FunctionType) -> Option<Multi> {
        match (sig.returns.len(), sig.return_var_arg) {
            (0, false) => None,
            (0, true) => Some((vec![], true)),
            _ => Some((sig.returns.clone().into_vec(), sig.return_var_arg)),
        }
    }

    #[inline]
    fn find_method(&self, ty: &Type, name: &str) -> Option<ObjectMethod> {
        let Type::Object { id: start, .. } = ty else {
            return None;
        };
        let mut id = *start;
        let mut class = self.object_class(ty);
        while let Some(c) = class {
            if let Some(found) = c.methods.iter().find(|m| m.name.as_ref() == name) {
                let mut found = found.clone();
                // a method's signature mentions the class's own variables, so it
                // is filled in with the arguments this application supplied
                if id == *start
                    && let Some(binding) = self.class_binding(ty)
                {
                    found.sig.params = found
                        .sig
                        .params
                        .iter()
                        .map(|t| binding.substitute(t))
                        .collect();
                    found.sig.returns = found
                        .sig
                        .returns
                        .iter()
                        .map(|t| binding.substitute(t))
                        .collect();
                }
                return Some(found);
            }
            class = match c.base {
                Some(base) => {
                    id = base;
                    self.class_by_id(ty, base)
                }
                None => None,
            };
        }
        None
    }

    fn infer_access(&mut self, path: &Path) -> Type {
        self.type_of_path(path)
    }

    /// Walks a path to the type it denotes, one suffix at a time
    fn type_of_path(&mut self, path: &Path) -> Type {
        match path {
            Path::Base((name, _)) => self.lookup_type(name.as_str()).unwrap_or(Type::Any),
            Path::Expr(expr) => self.infer_expr(expr),
            Path::Chain(receiver, suffix) => {
                let recv = self.type_of_path(receiver);
                self.suffix_type(&recv, suffix)
            }
        }
    }

    fn suffix_type(&mut self, recv: &Type, suffix: &PathSuffix) -> Type {
        match suffix {
            PathSuffix::Dot((name, _)) => self.member_type(recv, name),
            PathSuffix::Colon((name, _)) => self.method_type(recv, name),
            PathSuffix::Index(index) => self.index_type(recv, index),
            PathSuffix::NilCheck(_) | PathSuffix::TypeArgs(..) => Type::Any,
        }
    }

    /// `obj:method` used as a value keeps the method signature
    fn method_type(&mut self, recv: &Type, name: &str) -> Type {
        if let Some(m) = self.find_method(recv, name) {
            return Type::Function(Some(m.sig));
        }
        self.member_type(recv, name)
    }

    /// The class's own variables, filled in with the arguments this application
    /// supplied.
    ///
    /// A class written without arguments leaves its variables as variables, which is
    /// the honest answer: the field is a function of something nobody has said yet,
    /// and answering `any` here would make every check that reads it pass.
    fn class_binding(&self, recv: &Type) -> Option<solver::Solution> {
        let Type::Object { args, .. } = recv else {
            return None;
        };
        let class = self.object_class(recv)?;
        if class.type_params.is_empty() {
            return None;
        }
        Some(solver::Solution {
            bindings: class
                .type_params
                .iter()
                .cloned()
                .zip(args.iter().cloned())
                .collect(),
            ..Default::default()
        })
    }

    /// The type of a member of a class, with the class's own variables filled in.
    ///
    /// The two steps are in this order for a reason. The variables are recognised
    /// *before* the annotation is resolved, because a name that is a variable must
    /// not be looked up in the file -- there is nothing there to find, and an
    /// unresolved lookup answers `any`. The arguments go in afterwards, because it
    /// is the resolved type that mentions them.
    fn member_type_of(&mut self, recv: &Type, declared: &TypeDesc) -> Type {
        let names: Option<Vec<Box<str>>> = self
            .object_class(recv)
            .and_then(|class| (!class.type_params.is_empty()).then(|| class.type_params.to_vec()));
        let resolved = match names {
            Some(names) => {
                let names: Vec<&str> = names.iter().map(|n| &**n).collect();
                self.resolve_type(&normalize_generic_names(declared, &names))
            }
            None => self.resolve_type(declared),
        };
        match self.class_binding(recv) {
            Some(binding) => binding.substitute(&resolved),
            None => resolved,
        }
    }

    fn member_type(&mut self, recv: &Type, name: &str) -> Type {
        match recv {
            // A member read on a type parameter is a member read on whatever the
            // `where` bound says it may be. Without this the arm below answers
            // `any`, and a bound written to make `offset.x` checkable would
            // check nothing at all.
            Type::Param(var) => match self.lookup_type(&bound_key(var)) {
                Some(bound) => self.member_type(&bound, name),
                None => Type::Any,
            },
            Type::Object { .. } => {
                if let Some(ty) = self.class_member(recv, name) {
                    return self.member_type_of(recv, &ty);
                }
                if let Some(m) = self.find_method(recv, name) {
                    return Type::Function(Some(m.sig));
                }
                match recv {
                    Type::Object { mt: Some(mt), .. } => self.mt_index_type(mt, name),
                    _ => Type::Any,
                }
            }
            Type::TypeTable(fields, mt) => {
                let key = ConstValue::String(name.as_bytes().to_vec().into_boxed_slice());
                match fields.iter().find(|(k, _)| *k == key) {
                    Some((_, v)) => (**v).clone(),
                    None => match mt {
                        Some(mt) => self.mt_index_type(mt, name),
                        None => Type::Any,
                    },
                }
            }
            // a userdata is reached through the methods its type declares
            Type::UserData { name: ud } => self
                .userdata_types
                .get(ud.as_ref())
                .and_then(|ms| {
                    ms.iter()
                        .find(|(m, _)| m.as_ref() == name)
                        .map(|(_, t)| t.clone())
                })
                .unwrap_or(Type::Any),
            Type::Table(Some(k), Some(v), _) if matches!(**k, Type::String) => (**v).clone(),
            Type::Table(_, Some(v), _) => (**v).clone(),
            // a table that says nothing about its values lends the member
            // through its metatable's `__index`
            Type::Table(_, None, Some(mt)) => self.mt_index_type(mt, name),
            Type::Union(ts) => ts
                .iter()
                .map(|t| self.member_type(t, name))
                .find(|t| *t != Type::Any)
                .unwrap_or(Type::Any),
            _ => Type::Any,
        }
    }

    fn index_type(&mut self, recv: &Type, index: &Expr) -> Type {
        // a literal key picks one member, a computed one only gives the shape
        if let NodeKind::Literal(ConstValue::String(b)) = &index.0 {
            let name = String::from_utf8_lossy(b);
            if !matches!(recv, Type::TypeTable(..)) {
                return self.member_type(recv, &name);
            }
        }
        if let NodeKind::Literal(ConstValue::Int(i)) = &index.0
            && let Type::TypeTuple(items, _) = recv
            && let Some(ty) = usize::try_from(*i).ok().and_then(|i| items.get(i))
        {
            return ty.clone();
        }
        match recv {
            Type::Array(Some(inner)) => (**inner).clone(),
            Type::TypeTable(fields, _) => {
                let mut tys: Vec<Type> = fields.iter().map(|(_, v)| (**v).clone()).collect();
                tys.dedup();
                match tys.len() {
                    0 => Type::Any,
                    1 => tys.pop().unwrap_or(Type::Any),
                    _ => Type::Union(tys.into()),
                }
            }
            Type::Table(_, Some(v), _) => (**v).clone(),
            Type::Union(ts) => {
                let kept: Vec<Type> = ts
                    .iter()
                    .map(|t| self.index_type(t, index))
                    .filter(|t| *t != Type::Any)
                    .collect();
                match kept.len() {
                    0 => Type::Any,
                    _ => Type::from_union_kept(kept),
                }
            }
            _ => Type::Any,
        }
    }

    /// `require` runs the module and hands back its value, a module we cannot
    /// resolve statically stays `Any` instead of failing the whole file
    fn require_value(&mut self, args: &[Expr]) -> Type {
        let Some(Node(NodeKind::Literal(ConstValue::String(b)), _)) = args.first() else {
            return Type::Any;
        };
        let name = String::from_utf8_lossy(b);
        match self.resolve_module_type(&name) {
            Some(module) => module.value_type(),
            None => Type::Any,
        }
    }

    /// What `set_metatable` hands back: the value with the metatable on its type.
    fn set_metatable_value(&mut self, args: &[Expr]) -> Type {
        let val = args
            .first()
            .map(|a| self.infer_expr(a))
            .unwrap_or(Type::Any);
        let mt = match args.get(1).map(|a| self.infer_expr(a)) {
            Some(Type::Nil) => None,
            other => other,
        };
        match val {
            Type::Table(k, v, _) => Type::Table(k, v, mt.map(Box::new)),
            Type::Object {
                id,
                name,
                base,
                args: targs,
                module,
                ..
            } => Type::Object {
                id,
                name,
                base,
                args: targs,
                module,
                mt: mt.map(Box::new),
            },
            // a record keeps its own slot, and an undecided value has nothing to
            // say about one either
            Type::TypeTable(fields, _) => Type::TypeTable(fields, mt.map(Box::new)),
            Type::Any | Type::Union(_) | Type::Param(_) => val,
            other => {
                if let Some(arg) = args.first() {
                    self.err(
                        DukaSemanticError::TypeMismatchArg(
                            "a table or object".to_owned(),
                            other.to_string(),
                        ),
                        arg.1,
                    );
                }
                other
            }
        }
    }

    /// `get_metatable`: the metatable the type carries, or `nil`.
    fn metatable_of(&mut self, args: &[Expr]) -> Type {
        let val = args
            .first()
            .map(|a| self.infer_expr(a))
            .unwrap_or(Type::Any);
        match val {
            Type::Table(_, _, Some(mt))
            | Type::TypeTable(_, Some(mt))
            | Type::Object { mt: Some(mt), .. } => (*mt).clone(),
            Type::Table(..) | Type::TypeTable(..) | Type::Object { .. } => Type::Nil,
            _ => Type::Any,
        }
    }

    /// `mt.__index`: the member a metatable lends a table or class that lacks it.
    fn mt_index_type(&mut self, mt: &Type, name: &str) -> Type {
        let index = match mt {
            Type::TypeTable(fields, _) => fields
                .iter()
                .find(|(k, _)| *k == Self::field_key("__index"))
                .map(|(_, v)| (**v).clone()),
            Type::Object { .. } => self
                .class_member(mt, "__index")
                .map(|d| self.member_type_of(mt, &d)),
            _ => None,
        };
        match index {
            // `__index` is the table the member is actually on
            Some(Type::TypeTable(fields, _)) => fields
                .iter()
                .find(|(k, _)| *k == Self::field_key(name))
                .map(|(_, v)| (**v).clone())
                .unwrap_or(Type::Any),
            // as a function it can answer anything
            _ => Type::Any,
        }
    }

    fn field_key(name: &str) -> ConstValue {
        ConstValue::String(name.as_bytes().to_vec().into_boxed_slice())
    }

    /// The type a metamethod hands back, when the value declares one.
    fn meta_result(&mut self, ty: &Type, name: &str) -> Option<Type> {
        if let Type::Union(ts) = ty {
            return ts.iter().find_map(|t| self.meta_result(t, name));
        }
        if matches!(ty, Type::Object { .. })
            && let Some(m) = self.find_method(ty, name)
        {
            return Some(Self::first_return(&m.sig));
        }
        let mt = match ty {
            Type::Object { mt, .. } | Type::Table(_, _, mt) | Type::TypeTable(_, mt) => mt,
            _ => return None,
        };
        match self.member_type(mt.as_deref()?, name) {
            Type::Function(Some(ft)) => Some(Self::first_return(&ft)),
            _ => None,
        }
    }

    fn meta_binary(&mut self, l: &Type, r: &Type, name: &str) -> Option<Type> {
        if name.is_empty() {
            return None;
        }
        self.meta_result(l, name)
            .or_else(|| self.meta_result(r, name))
    }

    fn first_return(sig: &FunctionType) -> Type {
        // an unannotated metamethod says nothing about what it hands back
        sig.returns.first().cloned().unwrap_or(Type::Any)
    }

    /// `a.b.c = value`: the record `owner` gains `key`, then every record on the
    /// way back to the symbol is rebuilt with it
    fn assign_into(&mut self, owner: &Path, key: &str, value: &Type, span: Span) {
        if self.collect_mode {
            return;
        }
        let recv = self.type_of_path(owner);
        let obj = self.object_of_type(&recv);
        let member = obj
            .as_ref()
            .and_then(|o| self.object_class(o))
            .and_then(|o| o.members.iter().find(|m| m.name.as_ref() == key))
            .map(|m| m.ty.clone());
        if let Some(ty) = member {
            self.check_member_assign(&recv, &ty, value, span);
            return;
        }

        let Some(record) = self.record_after_write(&recv, key, value, span) else {
            return;
        };
        self.store_path(owner, record, span);
    }

    /// a record type with `key` set to `value`, the declared member type wins
    /// over a mismatching write
    fn record_after_write(
        &mut self,
        recv: &Type,
        key: &str,
        value: &Type,
        span: Span,
    ) -> Option<Type> {
        let ckey = ConstValue::String(key.as_bytes().to_vec().into_boxed_slice());
        let entry = || (ckey.clone(), Box::new(value.clone()));
        let mut mt = None;
        let mut fields = match recv {
            Type::Table(None, None, _) => vec![entry()],
            Type::TypeTable(fields, held) => {
                mt = held.clone();
                let mut fields = fields.clone();
                match fields.iter_mut().find(|(k, _)| *k == ckey) {
                    Some((_, slot)) => {
                        let merged = {
                            let declared = &**slot;
                            match (declared, value) {
                                (Type::TypeTable(..), Type::TypeTable(..)) => {
                                    Box::new(self.merge_record(declared, value))
                                }
                                (Type::Table(None, None, _) | Type::Any, _) => {
                                    Box::new(value.clone())
                                }
                                (declared, written) => {
                                    if !declared.accepts_value(written, None) {
                                        self.err(
                                            DukaSemanticError::TypeMismatchEqual(
                                                self.display_type(declared),
                                                self.display_type(written),
                                            ),
                                            span,
                                        );
                                    }
                                    slot.clone()
                                }
                            }
                        };
                        *slot = merged;
                    }
                    None => fields.push(entry()),
                }
                fields
            }
            _ => return None,
        };
        fields.dedup_by(|a, b| a.0 == b.0);
        Some(Type::TypeTable(fields, mt))
    }

    /// merges a written record into the declared one, a declared member type
    /// wins over the write that disagrees with it
    fn merge_record(&self, declared: &Type, written: &Type) -> Type {
        match (declared, written) {
            (Type::TypeTable(declared, _), Type::TypeTable(written, _)) => {
                let mut fields = declared.clone();
                for (key, value) in written {
                    match fields.iter_mut().find(|(k, _)| k == key) {
                        Some(slot) => {
                            *slot.1 = self.merge_record(&slot.1, value);
                        }
                        None => fields.push((key.clone(), value.clone())),
                    }
                }
                Type::TypeTable(fields, None)
            }
            // an open table carries no information, the write refines it
            (Type::Table(None, None, _) | Type::Any, _) => written.clone(),
            _ => declared.clone(),
        }
    }

    /// stores `value` into the record denoted by `path`, refreshing the symbol
    fn store_path(&mut self, path: &Path, value: Type, span: Span) {
        match path {
            Path::Base((name, nspan)) => self.declare(name, *nspan, value),
            Path::Chain(base, PathSuffix::Dot((key, _))) => {
                let recv = self.type_of_path(base);
                if let Some(record) = self.record_after_write(&recv, key, &value, span) {
                    self.store_path(base, record, span);
                }
            }
            _ => {}
        }
    }

    /// What a value written to a member has to satisfy.
    ///
    /// The declared type is read through the receiver it is written on, so a
    /// class applied to arguments checks a write against those arguments.
    fn check_member_assign(&mut self, recv: &Type, declared: &TypeDesc, value: &Type, span: Span) {
        let declared = self.member_type_of(recv, declared);
        if !declared.accepts_value(value, None) {
            self.err(
                DukaSemanticError::TypeMismatchEqual(
                    self.display_type(&declared),
                    self.display_type(value),
                ),
                span,
            );
        }
    }

    /// The tail statement of a module decides what `require` returns
    fn infer_tail(&mut self, tail: &Stmt) -> Option<Type> {
        match &tail.0 {
            NodeKind::Return(items, ..) => {
                let (tys, _) = self.infer_expr_list(items);
                tys.into_iter().next()
            }
            NodeKind::Expr(expr) => Some(self.infer_expr(expr)),
            NodeKind::Call(callee, args) => {
                let Node(NodeKind::Access(path), span) = &**callee else {
                    return None;
                };
                self.infer_call(path, *span, args)
                    .and_then(|(tys, _)| tys.into_iter().next())
            }
            _ => None,
        }
    }

    /// `ExportDesugarer` returns the export table when the module has no tail
    /// expression of its own, so the same rule decides the module value type
    fn module_value(&self, tail: Option<Type>) -> ModuleValue {
        let members = self.export_members.clone();
        let attributes = self.export_attributes.clone();
        let ty = match tail {
            Some(ty) => ty,
            None if members.is_empty() => Type::Table(None, None, None),
            None => Type::TypeTable(
                members
                    .iter()
                    .map(|(name, ty)| {
                        (
                            ConstValue::String(name.as_bytes().to_vec().into_boxed_slice()),
                            Box::new(ty.clone()),
                        )
                    })
                    .collect(),
                None,
            ),
        };
        ModuleValue {
            ty,
            members,
            attributes,
        }
    }

    /// Annotations are nilable by default, so a callable is often reached
    /// through a union: dig the signature out instead of losing it
    #[inline]
    fn as_function(ty: &Type) -> Option<FunctionType> {
        match ty {
            Type::Function(Some(ft)) => Some(ft.clone()),
            Type::Union(ts) => ts.iter().find_map(Self::as_function),
            _ => None,
        }
    }

    /// The variables a signature declares, as the solver wants them.
    ///
    /// The variables travel with the signature, so a builtin that lives in a
    /// module is as solvable as one called by name: the module has no name a
    /// call site can look the builtin up by, and two modules may have a member
    /// of the same name.
    fn signature_vars(ft: &FunctionType) -> Vec<solver::VarDecl> {
        ft.type_vars
            .iter()
            .map(|v| solver::VarDecl {
                name: v.name.clone(),
                bound: v.bound.as_deref().cloned(),
                default: v.default.as_deref().cloned(),
                // a builtin is written in no file a reader can point at
                span: Span::EMPTY,
                shape: if v.pack {
                    ParamMode::Pack
                } else {
                    ParamMode::Fixed
                },
                literal: v.literal,
            })
            .collect()
    }

    /// The argument types to constrain a call with, which is not always the type
    /// the argument was read as.
    ///
    /// A `literal` variable is filled in from what the argument *is*, so for the
    /// position that fills one the type is read off the argument itself. The
    /// read type has already widened a literal to the type of it, which is the
    /// one thing such a variable does not want.
    ///
    /// An argument with nothing more precise to give -- a variable, a call, a
    /// table -- is left as inferred, so `as_const(x)` answers whatever `x` is
    /// rather than refusing.
    fn literal_actuals(
        &self,
        params: &[Type],
        arg_types: &[Type],
        args: &[Expr],
        vars: &[solver::VarDecl],
    ) -> Vec<Type> {
        let mut actuals = arg_types.to_vec();
        for (idx, formal) in params.iter().enumerate() {
            let Some(name) = param_of(formal) else {
                continue;
            };
            let wants_literal = vars.iter().any(|v| v.literal && *v.name == **name);
            if !wants_literal {
                continue;
            }
            let Some(Node(NodeKind::Literal(cv), _)) = args.get(idx) else {
                continue;
            };
            if let Some(slot) = actuals.get_mut(idx) {
                *slot = Type::Literal(cv.clone());
            }
        }
        actuals
    }

    /// Checks the `where` list of a call, now that the variables are solved.
    ///
    /// This is the second run of the same clauses. The declaration pass read them
    /// with the variables still unknown, so a concept that mentioned one was an
    /// obligation rather than an answer; here the solution is substituted in and
    /// the question can finally be put.
    ///
    /// A concept that is still undecided after substitution is left alone. The
    /// solution does not always determine every variable -- one that only appears
    /// in a return type, say -- and refusing a call over a variable nobody passed
    /// would be worse than letting it through.
    fn discharge_where(
        &mut self,
        constraints: &[WhereClause],
        solution: &solver::Solution,
        call_span: Span,
    ) -> Vec<Box<str>> {
        let mut held = vec![];
        for clause in constraints {
            match clause {
                WhereClause::Bound((name, _), bound, _) => {
                    let Some(actual) = solution.bindings.get(name.as_str()) else {
                        continue;
                    };
                    let bound = self.resolve_type(bound);
                    if mentions_param(&bound) {
                        continue;
                    }
                    if !objects::accepts(&bound, actual, self.objects) {
                        self.err(
                            DukaSemanticError::WhereBoundViolated(
                                name.clone().into_boxed_str(),
                                bound.to_string(),
                                actual.to_string(),
                            ),
                            call_span,
                        );
                    }
                }
                WhereClause::Concept(cond, _) => {
                    // The solution goes into a scope of its own before the
                    // concept is read, rather than being substituted into the
                    // concept afterwards. Substitution cannot work here: the
                    // concept compares its operands, so `T == int` has to be
                    // read with `T` already answered -- substituting the answer
                    // into the result would compare `Param("T")` against `int`
                    // and then substitute into a `false` that is already
                    // decided. Without this the name does not resolve at all, and
                    // an unknown name in a type position is `any`.
                    self.types.push(std::collections::HashMap::new());
                    for (name, ty) in &solution.bindings {
                        self.types
                            .last_mut()
                            .expect("a scope was just pushed")
                            .insert(name.clone(), ty.clone());
                    }
                    let (ty, guarantees) =
                        self.resolve_type_and_constraints(&TypeDesc::Expr(cond.clone()));
                    self.types.pop();
                    if mentions_param(&ty) {
                        continue;
                    }
                    if matches!(
                        ty,
                        Type::Literal(ConstValue::Bool(false))
                            | Type::Literal(ConstValue::Nil)
                            | Type::Nil
                            | Type::Never
                    ) {
                        self.err(DukaSemanticError::WhereConceptFailed, call_span);
                        continue;
                    }
                    // the one clause of a `where` that is not a type
                    if let Some(name) = Self::concept_name(cond) {
                        held.push(name.to_owned().into_boxed_str());
                    }
                    // What the concept guaranteed goes into the flow. `Arg(i)`
                    // counts from where the concept was written, and the caller
                    // is the one that knows which argument that was and which
                    // variable it was passed as.
                    for g in guarantees {
                        self.apply(&g);
                    }
                }
                WhereClause::Bind(..) => {}
            }
        }
        held
    }

    /// Checks the arguments of a generic call once the variables are known: the
    /// formal type is substituted first, and anything still mentioning a
    /// variable or carrying no information is left alone
    fn check_generic_args(
        &mut self,
        params: &[Type],
        arg_types: &[Type],
        args: &[Expr],
        solution: &solver::Solution,
    ) {
        let params: Vec<Type> = params.iter().map(|p| solution.substitute(p)).collect();
        self.check_args(&params, arg_types, args, Span::EMPTY)
    }

    /// Records what a call promised against the variables it was passed.
    ///
    /// The guarantee was written about a type parameter -- `NonNil(T)` names `T`
    /// -- and the fact has to be about the variable that argument was bound to.
    /// The mapping is read off the signature rather than the expression: the
    /// parameter whose type mentions `T` is the one `T` solved to, so the
    /// position is found by asking which formal the variable appears in.
    ///
    /// The fact is not put into the flow here. `not_nil(x)` answers a bool, and
    /// what it guarantees about `x` only holds where that answer is read as
    /// true, so it waits in `pending` for a condition to pick it up.
    fn apply_ensure(
        &mut self,
        decl: &GenericDecl,
        params: &[Type],
        args: &[Expr],
        solution: &solver::Solution,
        call_span: Span,
    ) {
        let Some(expr) = decl.ensure.as_ref() else {
            return;
        };
        // The concept is read with the type parameters still called what they
        // were declared as. That is the name the guarantee comes back about, and
        // the solution is a separate question -- what the arguments were -- that
        // the mapping below answers.
        self.types.push(HashMap::new());
        for TypeParam((name, _), ..) in decl.params.iter() {
            self.types
                .last_mut()
                .expect("a scope was just pushed")
                .insert(
                    name.clone().into_boxed_str(),
                    Type::Param(name.clone().into()),
                );
        }
        let (_, guarantees) = self.resolve_type_and_constraints(&TypeDesc::Expr(expr.clone()));
        self.types.pop();
        for c in guarantees {
            let Some(Subject::TypeParam(t)) = c.subject() else {
                continue;
            };
            let Some(idx) = params.iter().position(|p| mentions_param_named(p, t)) else {
                continue;
            };
            let Some(Node(NodeKind::Access(p), _)) = args.get(idx) else {
                continue;
            };
            let Path::Base((name, _)) = &**p else {
                continue;
            };
            let c = subject_of(&c, Subject::Local(name.clone().into_boxed_str()));
            let c = resolve_against(&c, solution);
            self.pending.push((call_span, c));
        }
    }

    /// Discharges what a builtin says about its own call.
    ///
    /// A `where` list is a list of obligations the call has to meet, and a
    /// constraint is a fact about one of the arguments. Neither is a type, so
    /// neither travels in the signature; both are read here, where the call is
    /// and the arguments are known.
    fn apply_builtin_rules(
        &mut self,
        rules: &BuiltinRules,
        params: &[Type],
        args: &[Expr],
        solution: &solver::Solution,
        call_span: Span,
    ) {
        for clause in rules.where_clauses.iter() {
            match clause {
                WhereMeta::Bound { var, ty } => {
                    // the same check the `where U: Point` of a file does
                    let Some(actual) = solution.bindings.get(*var) else {
                        continue;
                    };
                    let bound: Type = ty.clone().into();
                    let bound = solution.substitute(&bound);
                    if mentions_param(&bound) {
                        continue;
                    }
                    if !objects::accepts(&bound, actual, self.objects) {
                        self.err(
                            DukaSemanticError::WhereBoundViolated(
                                (*var).into(),
                                bound.to_string(),
                                actual.to_string(),
                            ),
                            call_span,
                        );
                    }
                }
                WhereMeta::Bind { var, ty } => {
                    // a type-level binding: the name is what the type is, for
                    // the rest of the call
                    let ty: Type = ty.clone().into();
                    let ty = solution.substitute(&ty);
                    self.types
                        .last_mut()
                        .expect("a scope is open")
                        .insert((*var).into(), ty);
                }
                WhereMeta::Concept { name, args: cargs } => {
                    // A concept is a type-level expression read for its truth,
                    // and a builtin names one and gives it its arguments. It is
                    // read in a scope of its own where the solution's names are
                    // answered, because substitution cannot work here: the
                    // concept compares its operands.
                    self.types.push(HashMap::new());
                    for (n, ty) in &solution.bindings {
                        self.types
                            .last_mut()
                            .expect("a scope was just pushed")
                            .insert(n.clone(), ty.clone());
                    }
                    let values: Vec<TypeValue> = cargs
                        .iter()
                        .map(|a| {
                            let ty: Type = a.clone().into();
                            TypeValue::Type(solution.substitute(&ty))
                        })
                        .collect();
                    let ty = self.apply_concept(name, &values, call_span);
                    self.types.pop();
                    if mentions_param(&ty) {
                        continue;
                    }
                    if matches!(
                        ty,
                        Type::Literal(ConstValue::Bool(false))
                            | Type::Literal(ConstValue::Nil)
                            | Type::Nil
                            | Type::Never
                    ) {
                        self.err(DukaSemanticError::WhereConceptFailed, call_span);
                    }
                }
            }
        }
        for constraint in rules.constraints.iter() {
            let (arg, c) = match constraint {
                ConstraintMeta::Is { arg, ty } => {
                    (*arg, Constraint::Is(Subject::Arg(*arg), ty.clone().into()))
                }
                ConstraintMeta::IsNot { arg, ty } => (
                    *arg,
                    Constraint::IsNot(Subject::Arg(*arg), ty.clone().into()),
                ),
                ConstraintMeta::Sub { arg, ty } => {
                    (*arg, Constraint::Sub(Subject::Arg(*arg), ty.clone().into()))
                }
                ConstraintMeta::Instance { arg, ty } => (
                    *arg,
                    Constraint::Instance(Subject::Arg(*arg), ty.clone().into()),
                ),
                ConstraintMeta::Has { arg, field } => {
                    (*arg, Constraint::Has(Subject::Arg(*arg), (*field).into()))
                }
                ConstraintMeta::Never { arg } => (*arg, Constraint::Never(Subject::Arg(*arg))),
            };
            // The fact is about the argument at the call site, so it is about
            // the name the argument was written as: the flow tracks what it
            // knows about names, not about positions.
            let Some(Node(NodeKind::Access(p), _)) = args.get(arg) else {
                continue;
            };
            let Path::Base((name, _)) = &**p else {
                continue;
            };
            let c = subject_of(&c, Subject::Local(name.clone().into_boxed_str()));
            let c = resolve_against(&c, solution);
            self.pending.push((call_span, c));
        }
        let _ = params;
    }

    /// Reads a concept a builtin named: a type function applied to types, read
    /// for its truth.
    fn apply_concept(&mut self, name: &str, args: &[TypeValue], span: Span) -> Type {
        let callee = Node(
            NodeKind::Access(Box::new(Path::Base((name.into(), span)))),
            span,
        );
        let args: Box<[Node]> = args
            .iter()
            .map(|a| Node(NodeKind::TypeLit(TypeDesc::Pure(a.to_type())), span))
            .collect();
        let call = Node(NodeKind::Call(Box::new(callee), args), span);
        self.resolve_type(&TypeDesc::Expr(Box::new(call)))
    }

    /// Reads the calls a condition makes, so what they promised is waiting
    /// before the block they guard is walked.    ///
    /// Every call in the condition is read, not only the ones standing where
    /// truth flows through -- the rest are simply never picked up by
    /// `take_condition_facts`, and reading them here costs nothing but the walk.
    fn infer_condition_calls(&mut self, cond: &Expr) {
        struct Calls<'ctx, 'a>(&'ctx mut TypeCheckerCtx<'a>);
        impl Visitor for Calls<'_, '_> {
            fn visit_node(&mut self, expr: &Expr) {
                if let NodeKind::Call(callee, args) = &expr.0
                    && !self.0.pipe_operands.contains(&expr.1)
                    && let Node(NodeKind::Access(path), span) = &**callee
                {
                    self.0.infer_call(path, *span, args);
                }
            }
        }
        cond.visit(&mut Calls(self));
    }

    /// What a block guarded by a condition may take as known.
    ///
    /// `holds` says whether the condition is known to hold -- the block a
    /// condition guards sees it hold, and the `else` sees it fail -- and the
    /// walk follows the operators that carry truth to find what pins the
    /// outcome down: the calls, whose promises are waiting in `pending`, and
    /// comparisons with a literal, which are read here. A promise read the
    /// other way round is read as its negation, where it has one.
    fn condition_facts(&self, cond: &Expr, holds: bool) -> Vec<Constraint> {
        let mut items = vec![];
        truth_items(cond, holds, &mut items);
        // An element claim answers against the type as the batch finds it,
        // while a literal claim folds into what is already known when it is
        // applied. So the element claims go first and the literal ones carve
        // them down: `x ~= nil and x[0] == "a"` has to end with both read,
        // rather than with whichever was written first.
        let mut element_facts = vec![];
        let mut facts = vec![];
        for item in items {
            match item {
                Truth::Call(span, settles) => {
                    for (pending_span, c) in &self.pending {
                        if pending_span.start.at_char < span.start.at_char
                            || pending_span.end.at_char > span.end.at_char
                        {
                            continue;
                        }
                        facts.extend(if settles { Some(c.clone()) } else { negate(c) });
                    }
                }
                Truth::Const(name, value, is_equal) => {
                    // `==` is a method when either side is table-shaped, and a
                    // metatable may say a table equals anything, so only a type
                    // that cannot hold one is read
                    if !cannot_mask_equality(&self.declared_type(&name).unwrap_or(Type::Any)) {
                        continue;
                    }
                    // `nil` is not a literal type here: the union of a nilable
                    // annotation holds `Type::Nil` itself, and `x == nil` has to
                    // remove that member rather than a singleton that spells it
                    let ty = if value == ConstValue::Nil {
                        Type::Nil
                    } else {
                        Type::Literal(value)
                    };
                    facts.push(if is_equal {
                        Constraint::Is(Subject::Local(name), ty)
                    } else {
                        Constraint::IsNot(Subject::Local(name), ty)
                    });
                }
                Truth::Element(name, steps, value, is_equal) => {
                    let declared = self.declared_type(&name).unwrap_or(Type::Any);
                    let current = self
                        .fact(&name)
                        .filter(|f| objects::accepts(&declared, f, self.objects))
                        .unwrap_or(declared);
                    if let Some(narrowed) = narrow_by_path(&current, &steps, &value, is_equal)
                        && narrowed != Type::Never
                        && narrowed != current
                    {
                        element_facts.push(Constraint::Is(Subject::Local(name), narrowed));
                    }
                }
            }
        }
        element_facts.append(&mut facts);
        element_facts
    }

    /// Checks what was written against the parameters it was offered to: a
    /// parameter still mentioning a variable is the solver's, and the rest
    /// parameter is collected into a pack by the solver.
    fn check_args(&mut self, params: &[Type], arg_types: &[Type], args: &[Expr], call_span: Span) {
        for (idx, formal) in params.iter().enumerate() {
            let Some(actual) = arg_types.get(idx) else {
                break;
            };
            if mentions_param(formal) || !solver::Solver::has_info(actual) {
                continue;
            }
            let cv = match args.get(idx) {
                Some(Node(NodeKind::Literal(cv), _)) => Some(cv.clone()),
                _ => None,
            };
            if !objects::accepts_value(formal, actual, cv.as_ref(), self.objects) {
                self.err(
                    DukaSemanticError::TypeMismatchEqual(
                        self.display_type(formal),
                        self.display_type(actual),
                    ),
                    args.get(idx).map(|e| e.1).unwrap_or(call_span),
                );
            }
        }
    }

    /// Records what each type parameter of a generic call solved to, and what the
    /// `where` had to say.
    fn record_bindings(
        &mut self,
        call_span: Span,
        decl: &GenericDecl,
        solution: &solver::Solution,
        concepts: &[Box<str>],
    ) {
        if !self.collect_mode {
            let mut bindings: Vec<GenericBinding> = decl
                .params
                .iter()
                .filter_map(|param| {
                    let name = param.0.0.as_str();
                    let value = solution.bindings.get(name)?;
                    // A bound is written in one of two places -- the list after
                    // folded into the list by the parser, so one place to read
                    let bound = param
                        .1
                        .as_ref()
                        .map(|b| self.resolve_type(b))
                        .map(|b| solution.substitute(&b).to_string());
                    Some(GenericBinding {
                        name: name.to_owned().into_boxed_str(),
                        value: value.to_string().into_boxed_str(),
                        bound: bound.map(Into::into),
                        kind: BindingKind::Parameter,
                        value_type: Some(std::sync::Arc::new(value.clone())),
                    })
                })
                .collect();
            // `where type C = E` names a type in the signature above, so it
            // answers alongside the parameters.
            for clause in &decl.constraints {
                let WhereClause::Bind(names, value, _) = clause else {
                    continue;
                };
                let resolved = self.resolve_type(value);
                let substituted = solution.substitute(&resolved);
                // each name is reported with what its own place holds, which for a
                // pattern is not the whole value: `type [C, _] = [V, T]` says
                // `C = V`, and `_` says nothing at all because it binds nothing
                for bind in destructuring::bind(names, &substituted) {
                    let destructuring::Bind::Named(name, found) = bind else {
                        continue;
                    };
                    let found = found.unwrap_or_else(|| substituted.clone());
                    bindings.push(GenericBinding {
                        name: name.0.into_boxed_str(),
                        value: found.to_string().into_boxed_str(),
                        bound: None,
                        kind: BindingKind::TypeBinding,
                        value_type: Some(std::sync::Arc::new(found)),
                    });
                }
            }
            for name in concepts {
                bindings.push(GenericBinding {
                    name: name.clone(),
                    value: "true".into(),
                    bound: None,
                    kind: BindingKind::Concept,
                    value_type: None,
                });
            }
            if !bindings.is_empty() {
                self.generic_bindings.push((call_span, bindings));
            }
        }
    }

    /// Records what the type variables of a signature solved to.
    ///
    /// A function written in the language has a declaration beside its
    /// signature, and that is where the `where` obligations live. A member or a
    /// value only has the signature, so the variables are read from it.
    fn record_signature_bindings(
        &mut self,
        call_span: Span,
        ft: &FunctionType,
        solution: &solver::Solution,
    ) {
        if self.collect_mode || ft.type_vars.is_empty() {
            return;
        }
        let bindings: Vec<GenericBinding> = ft
            .type_vars
            .iter()
            .filter_map(|var| {
                let value = solution.bindings.get(&var.name)?;
                Some(GenericBinding {
                    name: var.name.clone(),
                    value: value.to_string().into_boxed_str(),
                    bound: var
                        .bound
                        .as_deref()
                        .map(|bound| solution.substitute(bound).to_string().into_boxed_str()),
                    kind: BindingKind::Parameter,
                    value_type: Some(std::sync::Arc::new(value.clone())),
                })
            })
            .collect();
        if !bindings.is_empty() {
            self.generic_bindings.push((call_span, bindings));
        }
    }

    /// The name of an applied concept, which is what a reader wants: a concept
    /// is an expression, not a type, so there is nothing to substitute into.
    fn concept_name(cond: &Expr) -> Option<&str> {
        let path = match &cond.0 {
            NodeKind::Call(callee, _) => match &callee.0 {
                NodeKind::Access(path) => path,
                _ => return None,
            },
            NodeKind::Access(path) => path,
            _ => return None,
        };
        match &**path {
            Path::Base((name, _)) => Some(name.as_str()),
            _ => None,
        }
    }

    /// The solver reports diagnostics against the type parameter's own span,
    /// which is its declaration. That is where the reader cannot act, so every
    /// one of them is re-spanned onto the call that produced it.
    fn report_solution(&mut self, solution: &solver::Solution) {
        for diagnostic in &solution.diagnostics {
            match diagnostic {
                solver::Diagnostic::Ambiguous { name, .. } => {
                    self.err(
                        DukaSemanticError::TypeParamUnresolved(name.clone()),
                        solution.span,
                    );
                }
                solver::Diagnostic::BoundViolated {
                    name,
                    bound,
                    candidate,
                    ..
                } => {
                    self.err(
                        DukaSemanticError::TypeParamBoundViolated(
                            name.clone(),
                            bound.to_string(),
                            candidate.to_string(),
                        ),
                        solution.span,
                    );
                }
                solver::Diagnostic::ArityMismatch {
                    expected, given, ..
                } => {
                    self.err(
                        DukaSemanticError::TypeArgArityMismatch(*expected, *given),
                        solution.span,
                    );
                }
            }
        }
    }

    /// Answers a call to a signature that declares type parameters.
    ///
    /// The variables are the ones the signature carries, so this is the same
    /// whether the function was written in a file or handed over by the
    /// runtime, and whether it was called by name or reached through a member.
    /// `given` are the type arguments written at the call site, which take
    /// precedence over what the arguments imply.
    fn solve_signature(
        &mut self,
        ft: &FunctionType,
        arg_types: &[Type],
        args: &[Expr],
        call_span: Span,
        given: Vec<Type>,
    ) -> Option<Multi> {
        if ft.type_vars.is_empty() {
            return None;
        }
        let vars = Self::signature_vars(ft);
        let mut solver = solver::Solver::new(vars.clone()).with_objects(self.objects);
        let actuals = self.literal_actuals(&ft.params, arg_types, args, &vars);
        solver.constrain_call(&ft.params, &actuals, call_span);
        // An explicit type argument does not excuse the value written beside it.
        // Without one the call is read the way inference reads it, which leaves
        // what the arguments widen to alone.
        let explicit = !given.is_empty();
        let solution = solver.solve(given, call_span);
        self.report_solution(solution);
        if explicit {
            self.check_generic_args(&ft.params, &actuals, args, solution);
        }
        // A member reached through a value has no declaration beside its
        // signature, so the solved variables are read off the signature itself.
        // That is what lets a hover on `push` say what `T` was this time.
        self.record_signature_bindings(call_span, ft, solution);
        Some(solver.returns_of(ft))
    }

    /// What a callish call means, which is not a call at all.
    ///
    /// `spawn` starts a coroutine and hands back a handle to it, `go` switches
    /// to one and `yield` gives control back from the running one. A coroutine
    /// is a thing of its own, so it has a type of its own.
    fn infer_callish(&mut self, callish: &str, args: &[Expr], call_span: Span) -> Multi {
        match callish {
            ccallish::SPAWN => {
                if let Some(arg) = args.first() {
                    let ty = self.infer_expr(arg);
                    if !matches!(ty, Type::Function(..) | Type::Any) {
                        self.err(
                            DukaSemanticError::TypeMismatchArg(
                                "a function".to_owned(),
                                ty.to_string(),
                            ),
                            arg.1,
                        );
                    }
                }
                (vec![Type::Coroutine], false)
            }
            ccallish::GO => {
                if let Some(arg) = args.first() {
                    let ty = self.infer_expr(arg);
                    if !matches!(ty, Type::Coroutine | Type::Any) {
                        self.err(
                            DukaSemanticError::TypeMismatchArg(
                                "a coroutine".to_owned(),
                                ty.to_string(),
                            ),
                            arg.1,
                        );
                    }
                }
                (vec![], false)
            }
            // what a coroutine yields is sent to whoever resumed it, and the
            // runtime hands the values back where they are asked for
            ccallish::YIELD => (vec![], true),
            _ => {
                let _ = call_span;
                (vec![Type::Any], false)
            }
        }
    }

        fn infer_call(&mut self, path: &Path, call_span: Span, args: &[Expr]) -> Option<Multi> {
            if let Path::Base((name, _)) = path
                && name.as_str() == cpar::REQUIRE
            {
                return Some((vec![self.require_value(args)], false));
            }
            // A metatable travels in the value's type, so these two are read
            // here rather than left to a signature that cannot name it.
            if let Path::Base((name, _)) = path {
                match name.as_str() {
                    cpar::SET_METATABLE => {
                        return Some((vec![self.set_metatable_value(args)], false));
                    }
                    cpar::GET_METATABLE => return Some((vec![self.metatable_of(args)], false)),
                    _ => {}
                }
            }
        // `spawn`, `go` and `yield` are not functions: they are what the
        // runtime does with a coroutine, and they are typed as themselves.
        if let Some(callish) = path.is_callish() {
            return Some(self.infer_callish(callish, args, call_span));
        }
        let (arg_types, args_var_arg) = self.infer_expr_list(args);
        // What the bindings are recorded against is the name that was called,
        // not the span of the call expression. A reader asks the question while
        // pointing at `B`, and the span the expression carries is the whole
        // `B(...)` -- which does contain the name, but the callee expression
        // hands over only its own token, which is the one after the arguments.
        let call_span = call_span.cover(path.get_span());
        match path {
            Path::Base((name, _)) => {
                let sig = self.lookup_type(name)?;
                let Some(ft) = Self::as_function(&sig) else {
                    return Some((vec![Type::Any], false));
                };
                self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);

                let decl = self.generic_fns.get(name.as_str()).cloned();
                let rules = self.builtin_rules.get(name.as_str()).cloned();

                if ft.type_vars.is_empty() && decl.is_none() {
                    self.check_args(&ft.params, &arg_types, args, call_span);

                    if let Some(rules) = &rules {
                        let mut solver = solver::Solver::new(vec![]);
                        let solution = solver.solve(vec![], call_span);

                        self.apply_builtin_rules(rules, &ft.params, args, solution, call_span);
                    }

                    return Some((ft.returns.into_vec(), ft.return_var_arg));
                }

                let vars = Self::signature_vars(&ft);
                let mut solver = solver::Solver::new(vars.clone()).with_objects(self.objects);
                let actuals = self.literal_actuals(&ft.params, &arg_types, args, &vars);
                solver.constrain_call(&ft.params, &actuals, call_span);
                // the tail was already read value-style; offer it to the pack as one tuple
                if let Some(bind) = &ft.var_arg_bind
                    && vars
                        .iter()
                        .any(|v| v.name.as_ref() == bind.as_ref() && v.shape == ParamMode::Pack)
                {
                    let start = ft.params.len().min(arg_types.len());
                    let items = arg_types[start..].to_vec();
                    let formal = Type::Param(bind.clone());
                    solver.constrain_tail(
                        &formal,
                        &Type::TypeTuple(items.into(), args_var_arg),
                        call_span,
                    );
                }
                let solution = solver.solve(vec![], call_span);

                self.report_solution(solution);
                if let Some(decl) = &decl {
                    // a declaration beside the signature is checked against
                    // what the variables solved to, so the argument has to fit
                    // the type it was given
                    self.check_generic_args(&ft.params, &actuals, args, solution);
                    self.apply_ensure(decl, &ft.params, args, solution, call_span);
                    // the `where` is discharged first because a concept is one of the
                    // obligations the answer has to list, and reading it is the only
                    // way to know which of them this call actually discharged
                    let concepts = self.discharge_where(&decl.constraints, solution, call_span);
                    self.record_bindings(call_span, decl, solution, &concepts);
                } else {
                    // a signature on its own is checked as it is written: the
                    // parameters that name a variable are the solver's business
                    self.check_args(&ft.params, &actuals, args, call_span);
                }
                if let Some(rules) = rules {
                    self.apply_builtin_rules(&rules, &ft.params, args, solution, call_span);
                }

                Some(solver.returns_of(&ft))
            }
            Path::Chain(receiver, PathSuffix::TypeArgs(ty_args, _)) => {
                let given: Vec<Type> = ty_args.iter().map(|a| self.resolve_type(a)).collect();
                // `push.<string>(...)` names a member, not a top-level function:
                // the receiver is read like any other member call and the written
                // type arguments are handed to the same solver.
                match receiver.as_ref() {
                    Path::Chain(inner, PathSuffix::Dot((name, name_span))) => self.member_call(
                        inner,
                        name,
                        *name_span,
                        call_span,
                        &arg_types,
                        args_var_arg,
                        args,
                        true,
                        given,
                    ),
                    Path::Chain(inner, PathSuffix::Colon((name, name_span))) => self.member_call(
                        inner,
                        name,
                        *name_span,
                        call_span,
                        &arg_types,
                        args_var_arg,
                        args,
                        false,
                        given,
                    ),
                    Path::Base((fname, _)) => {
                        let sig = self.lookup_type(fname)?;
                        let Some(ft) = Self::as_function(&sig) else {
                            return Some((vec![Type::Any], false));
                        };
                        self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);
                        // The declaration beside the signature is only what the
                        // signature cannot say: the `where` list and the postcondition.
                        let decl = self.generic_fns.get(fname.as_str()).cloned();
                        if ft.type_vars.is_empty() && decl.is_none() {
                            self.check_args(&ft.params, &arg_types, args, call_span);
                            return Some((ft.returns.into_vec(), ft.return_var_arg));
                        }
                        // The variables are the ones the signature declares, and they
                        // are solved for the same way wherever the signature came from.
                        let vars = Self::signature_vars(&ft);
                        let mut solver =
                            solver::Solver::new(vars.clone()).with_objects(self.objects);
                        let actuals = self.literal_actuals(&ft.params, &arg_types, args, &vars);
                        let solution = solver.solve(given, call_span);
                        self.report_solution(solution);
                        if let Some(decl) = &decl {
                            // a declaration beside the signature is checked against
                            // what the variables solved to, so the argument has to fit
                            // the type it was given
                            self.check_generic_args(&ft.params, &actuals, args, solution);
                            self.apply_ensure(decl, &ft.params, args, solution, call_span);
                            // the `where` is discharged first because a concept is one of the
                            // obligations the answer has to list, and reading it is the only
                            // way to know which of them this call actually discharged
                            let concepts =
                                self.discharge_where(&decl.constraints, solution, call_span);
                            self.record_bindings(call_span, decl, solution, &concepts);
                        } else {
                            // a signature on its own is checked as it is written: the
                            // parameters that name a variable are the solver's business
                            self.check_args(&ft.params, &actuals, args, call_span);
                        }

                        Some(solver.returns_of(&ft))
                    }
                    _ => Some((vec![Type::Any], false)),
                }
            }
            Path::Chain(receiver, PathSuffix::Colon((mname, mspan))) => {
                let (mname, mspan) = (String::from(mname), *mspan);
                self.member_call(
                    receiver,
                    &mname,
                    mspan,
                    call_span,
                    &arg_types,
                    args_var_arg,
                    args,
                    false,
                    vec![],
                )
            }

            Path::Chain(receiver, PathSuffix::Dot((name, name_span))) => {
                let (name, name_span) = (String::from(name), *name_span);
                if name == csugar::NEW_FUNC
                    && let Some(obj) = self.receiver_object(receiver)
                {
                    // a factory hands back the class it was read off, whatever
                    // module declared it -- and the arguments are the ones its
                    // `init` takes
                    if let Some(init) = self.find_method(&obj, csugar::INIT_FUNC) {
                        self.check_call_arity(
                            &init.sig,
                            arg_types.len(),
                            args_var_arg,
                            call_span,
                        );
                        self.check_args(&init.sig.params, &arg_types, args, call_span);
                    }
                    return Some((vec![obj], false));
                }
                self.member_call(
                    receiver,
                    &name,
                    name_span,
                    call_span,
                    &arg_types,
                    args_var_arg,
                    args,
                    true,
                    vec![],
                )
            }

            Path::Expr(callee) => {
                let Type::Function(Some(ft)) = self.infer_expr(callee) else {
                    return None;
                };
                self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);
                // A signature that declares variables is solved for at the call
                // site, wherever it came from -- including a function reached
                // through a value rather than by name.
                if let Some(multi) = self.solve_signature(&ft, &arg_types, args, call_span, vec![])
                {
                    return Some(multi);
                }
                self.check_args(&ft.params, &arg_types, args, call_span);
                Some((ft.returns.into_vec(), ft.return_var_arg))
            }

            _ => None, // FIXME,
        }
    }

    /// `recv.name(...)`: an object method when the receiver is an object, a
    /// function stored in a record or table otherwise
    fn member_call(
        &mut self,
        receiver: &Path,
        name: &str,
        name_span: Span,
        call_span: Span,
        arg_types: &[Type],
        args_var_arg: bool,
        args: &[Expr],
        receiver_written: bool,
        given: Vec<Type>,
    ) -> Option<Multi> {
        if let Some(obj) = self.receiver_object(receiver)
            && let Some(m) = self.find_method(&obj, name)
        {
            // a method of a class in another module is recorded against that
            // module's table, which is where its `ObjectType` lives
            if let Type::Object {
                id, module: None, ..
            } = &obj
            {
                self.links.push(MethodLink {
                    call_span,
                    name_span,
                    decl_span: m.span,
                    owner: *id,
                });
            }
            // a method called through a class name is measured with the
            // receiver in front of the parameters it was written with
            // a static method is a plain function that happens to sit on the class, so
            // only an instance method called with `.` has the receiver written
            // out in front of its parameters
            let sig = Self::signature_as_called(&m.sig, receiver_written && !m.is_static);
            self.check_call_arity(&sig, arg_types.len(), args_var_arg, call_span);
            self.check_args(&sig.params, arg_types, args, call_span);
            if let Some(multi) = self.solve_signature(&sig, arg_types, args, call_span, given) {
                return Some(multi);
            }
            return self.return_type_of(&m.sig);
        }
        let recv = self.type_of_path(receiver);
        let Some(ft) = Self::as_function(&self.member_type(&recv, name)) else {
            return None;
        };
        // A userdata's methods take the receiver as their first argument, and
        // the `:` form hands the receiver over itself: what is written after the
        // colon is the rest of the arguments.
        let ft = if !receiver_written && matches!(recv, Type::UserData { .. }) {
            FunctionType {
                params: ft.params.iter().skip(1).cloned().collect(),
                ..ft
            }
        } else {
            ft
        };
        self.check_call_arity(&ft, arg_types.len(), args_var_arg, call_span);
        self.check_args(&ft.params, arg_types, args, call_span);
        // A member that declares type parameters is solved for like any other
        // call: `array.pop` answers the element type of the array it was given.
        if let Some(multi) = self.solve_signature(&ft, arg_types, args, call_span, given) {
            return Some(multi);
        }
        Some((ft.returns.into_vec(), ft.return_var_arg))
    }

    /// A signature as the call site sees it, which is not always the signature as
    /// it was written.
    ///
    /// An instance method called with `.` is handed the receiver as its first
    /// argument -- `A.init(self, a)` is how a subclass runs its parent's
    /// initializer -- so its parameters are measured one further along than the
    /// same method called with `:`. The receiver has no type: it is the instance
    /// the method works on, not a value handed to it.
    fn signature_as_called(sig: &FunctionType, receiver_written: bool) -> FunctionType {
        if !receiver_written {
            return sig.clone();
        }
        FunctionType {
            params: [Type::Any]
                .into_iter()
                .chain(sig.params.iter().cloned())
                .collect(),
            ..sig.clone()
        }
    }

    fn check_call_arity(&mut self, ft: &FunctionType, got: usize, open_ended: bool, span: Span) {
        if open_ended || !self.final_args {
            return;
        }
        // a signature carries only the named parameters, the rest parameter is
        // `var_arg` and says nothing is required beyond them
        let required = ft.params.len();
        // an argument with no parameter to land in is a mistake in either
        // direction: too few leaves the call short, and too many leaves the
        // surplus with nowhere to go, so it is dropped at runtime rather than
        // refused -- which is why it has to be caught here
        if got < required || (got > required && !ft.var_arg) {
            self.err(
                DukaSemanticError::TypeMismatchArg(argument_count(required), argument_count(got)),
                span,
            );
        }
    }
}

/// What a generic signature declared: the variables it solves for, and the
/// `where` list that says what the solution has to satisfy.
///
/// Both halves travel together because they are only useful together. The
/// variables are solved at the call site, and the obligations that were left
/// over from the declaration can only be discharged once the solution is known,
/// which needs the clauses that produced them.
#[derive(Debug, Clone)]
struct GenericDecl {
    params: Box<[TypeParam]>,
    constraints: Box<[WhereClause]>,
    /// The postcondition as written, read at each call while the type parameters
    /// still have their names -- the solution is a separate question, answered
    /// by `apply_ensure`.
    ensure: Option<Box<Expr>>,
}

impl std::ops::Deref for GenericDecl {
    type Target = [TypeParam];
    fn deref(&self) -> &[TypeParam] {
        &self.params
    }
}

/// The name a `where` bound is filed under inside a type frame.
///
/// A prefix that cannot be written, so a `where U: Point` cannot shadow a
/// parameter the program actually declared. `Type::Param` already reserves a
/// prefix for its own placeholders, so the two cannot collide either.
fn bound_key(param: &str) -> String {
    format!("{BOUND_PARAM_PREFIX}{param}")
}

const BOUND_PARAM_PREFIX: &str = "\u{1}bound\u{1}";

/// Whether a concept names one of these type parameters.
///
/// A bare name in a type position can only appear as a `TypeLit`, so that is the
/// one place a parameter can hide. The walk is over the whole expression because
/// a concept can nest a record or a call inside itself, and it looks at the
/// concept rather than at its answer: `T == int` answers `false`, which says
/// nothing about whether `T` was still undecided.
fn type_expr_names_type_param(expr: &Expr, params: &[Box<str>]) -> bool {
    struct Walk<'a>(&'a [Box<str>], bool);
    impl Visitor for Walk<'_> {
        fn visit_node(&mut self, expr: &Expr) {
            if let NodeKind::TypeLit(td) = &expr.0 {
                self.1 |= type_desc_names_any(td, self.0);
            }
        }
    }
    let mut w = Walk(params, false);
    expr.visit(&mut w);
    w.1
}

fn type_desc_names_any(td: &TypeDesc, params: &[Box<str>]) -> bool {
    /// Looks for one of the type parameters among the names a type holds.
    ///
    /// It reads the names the walk hands out rather than listing the children by
    /// hand, so a type written as an expression is searched like any other. Before,
    /// a concept mentioning a parameter through `F(T)` was not seen as mentioning
    /// one at all -- the child list had no expression in it -- so the concept was
    /// answered as though it were about nothing, and a bound that only held
    /// because of it went unrecorded.
    struct Walk<'a>(&'a [Box<str>], bool);
    impl Visitor for Walk<'_> {
        fn visit_type_name(&mut self, name: &str, _span: &Span) {
            if self.0.iter().any(|p| p.as_ref() == name) {
                self.1 = true;
            }
        }
    }
    let mut w = Walk(params, false);
    td.visit(&mut w);
    w.1
}

/// Whether a type still mentions a type variable, which only happens inside a
/// generic body where the variable is bound at the call site
fn mentions_param(ty: &Type) -> bool {
    match ty {
        Type::Param(_) => true,
        Type::Array(inner) => inner.as_deref().is_some_and(mentions_param),
        Type::Table(k, v, mt) => {
            k.as_deref().is_some_and(mentions_param)
                || v.as_deref().is_some_and(mentions_param)
                || mt.as_deref().is_some_and(mentions_param)
        }
        Type::Union(ts) => ts.iter().any(mentions_param),
        Type::TypeTuple(ts, _) => ts.iter().any(mentions_param),
        Type::TypeTable(fields, _) => fields.iter().any(|(_, t)| mentions_param(t)),
        Type::Object { args, .. } => args.iter().any(mentions_param),
        Type::Function(Some(ft)) => ft
            .params
            .iter()
            .chain(ft.returns.iter())
            .any(mentions_param),
        Type::Rec(inner) => mentions_param(inner),
        _ => false,
    }
}

/// The type variable a parameter is written as, if it is one; `x: C` arrives
/// as `C?` because a written annotation is nilable by default.
fn param_of(ty: &Type) -> Option<&Box<str>> {
    match ty {
        Type::Param(name) => Some(name),
        Type::Union(us) => us.iter().find_map(param_of),
        _ => None,
    }
}

/// `a.b.c = v` splits into the record owner `a.b` and the member `c`
fn assign_target(target: &Path) -> Option<(&Path, &str, Span)> {
    let Path::Chain(owner, PathSuffix::Dot((key, key_span))) = target else {
        return None;
    };
    match owner.as_ref() {
        Path::Base(_) | Path::Chain(..) => Some((owner, key, *key_span)),
        _ => None,
    }
}

/// Names an `export` statement publishes to the importer
fn exported_names(stmt: &NodeKind) -> Vec<(Box<str>, Span)> {
    match stmt {
        NodeKind::Define(names, ..) => names
            .iter()
            .map(|(((name, span), _, _), _)| (name.clone().into_boxed_str(), *span))
            .collect(),
        NodeKind::FunctionDecl(Path::Base((name, span)), ..) => {
            vec![(name.clone().into_boxed_str(), *span)]
        }
        NodeKind::Function(..) => vec![],
        // A class is a value like any other as far as an importer is concerned:
        // `m.Point.new(1)` is a call on something `m.Point` names. It was left out
        // here, which is the only reason `require("m").Point` read as nothing at
        // all rather than as a class.
        NodeKind::Object(obj) => {
            vec![(obj.name.0.clone().into_boxed_str(), obj.name.1)]
        }
        NodeKind::Assign(targets, _) => targets
            .iter()
            .filter_map(|target| match target {
                Path::Base((name, span)) => Some((name.clone().into_boxed_str(), *span)),
                Path::Chain(base, PathSuffix::Dot((name, span))) => match base.as_ref() {
                    Path::Base((base, _)) if base == name => {
                        Some((name.clone().into_boxed_str(), *span))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

/// The metamethod that stands in for a binary operator.
fn binop_metamethod(op: &BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::Add => MetaMethod::Add.name(),
        BinOp::Sub => MetaMethod::Sub.name(),
        BinOp::Multiply => MetaMethod::Mul.name(),
        BinOp::Divide => MetaMethod::Div.name(),
        BinOp::IDivide => MetaMethod::IDiv.name(),
        BinOp::Mod => MetaMethod::Mod.name(),
        BinOp::Pow => MetaMethod::Pow.name(),
        BinOp::Concat => MetaMethod::Concat.name(),
        BinOp::BitAnd => MetaMethod::BAnd.name(),
        BinOp::BitOr => MetaMethod::BOr.name(),
        BinOp::BitXor => MetaMethod::BXor.name(),
        BinOp::ShiftL => MetaMethod::ShL.name(),
        BinOp::ShiftR => MetaMethod::ShR.name(),
        _ => return None,
    })
}

/// Whether a value is the literal `nil`, or absent because the list ran out.
///
/// Both give a name nothing to be: `local a = nil` and `local a` say the name
/// may hold something later, not that it holds nothing but nil. A value the
/// flow narrowed down to nil is not this, and keeps the nil it was proved to be.
fn nil_literal_or_none(expr: Option<&Expr>) -> bool {
    match expr {
        None => true,
        Some(Node(NodeKind::Literal(ConstValue::Nil), _)) => true,
        _ => false,
    }
}

fn argument_count(n: usize) -> String {
    format!("{n} argument{}", if n == 1 { "" } else { "s" })
}

pub(crate) fn substitute_params(ty: &Type, subst: &HashMap<Box<str>, Type>) -> Type {
    match ty {
        Type::TypeTable(t, mt) => Type::TypeTable(
            t.iter()
                .map(|(k, v)| (k.clone(), Box::new(substitute_params(v, subst))))
                .collect(),
            mt.as_ref()
                .map(|m| Box::new(substitute_params(m, subst))),
        ),
        Type::TypeTuple(v, rest) => Type::TypeTuple(
            v.iter().map(|t| substitute_params(t, subst)).collect(),
            *rest,
        ),
        //Type::TypeTable()
        Type::Param(name) => subst
            .get(name)
            .cloned()
            .unwrap_or_else(|| Type::Param(name.clone())),
        Type::Array(Some(inner)) => Type::Array(Some(Box::new(substitute_params(inner, subst)))),
        Type::Array(None) => Type::Array(None),
        Type::Table(k, v, mt) => Type::Table(
            k.as_deref().map(|k| Box::new(substitute_params(k, subst))),
            v.as_deref().map(|v| Box::new(substitute_params(v, subst))),
            mt.as_deref().map(|m| Box::new(substitute_params(m, subst))),
        ),
        Type::Union(ts) => Type::Union(ts.iter().map(|t| substitute_params(t, subst)).collect()),
        Type::Object {
            id,
            name,
            base,
            args,
            module,
            mt,
        } => Type::Object {
            id: *id,
            name: name.clone(),
            base: *base,
            args: args.iter().map(|t| substitute_params(t, subst)).collect(),
            module: module.clone(),
            mt: mt.as_deref().map(|m| Box::new(substitute_params(m, subst))),
        },
        Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
            params: ft
                .params
                .iter()
                .map(|t| substitute_params(t, subst))
                .collect(),
            var_arg_bind: ft.var_arg_bind.clone(),
            returns: ft
                .returns
                .iter()
                .map(|t| substitute_params(t, subst))
                .collect(),
            var_arg: ft.var_arg,
            return_var_arg: ft.return_var_arg,
            return_var_arg_bind: ft.return_var_arg_bind.clone(),
            type_vars: ft.type_vars.clone(),
        })),
        Type::Rec(inner) => Type::Rec(Box::new(substitute_params(inner, subst))),
        other => other.clone(),
    }
}

/// Substitution that stops at a `Rec`. A recursive type is the boundary of
/// itself: the back edge inside its body is already bound, so a substitution
/// made from outside must not reach in. Without that boundary every projection
/// would deepen the type instead of answering with the same recursive type,
/// which is what `LList(int)[1][1]` should be.
pub(crate) fn substitute_back_edges(ty: &Type, subst: &HashMap<Box<str>, Type>) -> Type {
    match ty {
        Type::TypeTable(t, mt) => Type::TypeTable(
            t.iter()
                .map(|(k, v)| (k.clone(), Box::new(substitute_back_edges(v, subst))))
                .collect(),
            mt.as_ref()
                .map(|m| Box::new(substitute_back_edges(m, subst))),
        ),
        Type::TypeTuple(v, rest) => Type::TypeTuple(
            v.iter().map(|t| substitute_back_edges(t, subst)).collect(),
            *rest,
        ),
        Type::Param(name) => subst
            .get(name)
            .cloned()
            .unwrap_or_else(|| Type::Param(name.clone())),
        Type::Array(Some(inner)) => {
            Type::Array(Some(Box::new(substitute_back_edges(inner, subst))))
        }
        Type::Array(None) => Type::Array(None),
        Type::Table(k, v, mt) => Type::Table(
            k.as_deref()
                .map(|k| Box::new(substitute_back_edges(k, subst))),
            v.as_deref()
                .map(|v| Box::new(substitute_back_edges(v, subst))),
            mt.as_deref()
                .map(|m| Box::new(substitute_back_edges(m, subst))),
        ),
        Type::Union(ts) => {
            Type::Union(ts.iter().map(|t| substitute_back_edges(t, subst)).collect())
        }
        Type::Object {
            id,
            name,
            base,
            args,
            module,
            mt,
        } => Type::Object {
            id: *id,
            name: name.clone(),
            base: *base,
            args: args
                .iter()
                .map(|t| substitute_back_edges(t, subst))
                .collect(),
            module: module.clone(),
            mt: mt
                .as_deref()
                .map(|m| Box::new(substitute_back_edges(m, subst))),
        },
        Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
            params: ft
                .params
                .iter()
                .map(|t| substitute_back_edges(t, subst))
                .collect(),
            var_arg_bind: ft.var_arg_bind.clone(),
            returns: ft
                .returns
                .iter()
                .map(|t| substitute_back_edges(t, subst))
                .collect(),
            var_arg: ft.var_arg,
            return_var_arg: ft.return_var_arg,
            return_var_arg_bind: ft.return_var_arg_bind.clone(),
            type_vars: ft.type_vars.clone(),
        })),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use duka_shared::docs::{
        BuiltinInfo, DocType, MetaInfo, MetaItemInfo, MetaScope, ReturnMeta, WhereMeta,
    };
    use duka_shared::dtype::Type;
    use duka_shared::types::{DukaAnalyzer, DukaLexer, DukaParser, SourceName};

    use crate::analyzer::DukaTypeChecker;

    use crate::{
        analyzer::AnalyzerData, analyzer::BuiltinRules, analyzer::ScopeAnalyzer,
        analyzer::TypeAnalyzer, analyzer::TypeChecker, lexer::LexerWithMacro, parser::Parser,
        parser::ast::DukaChunk,
    };

    /// The errors a source produces when a builtin is provided that says what
    /// its own type is.
    ///
    /// The builtin's type is read where the scope is, so the injection has to
    /// be part of the run: a name the builtin holds is a name the scope has to
    /// know.
    fn check_with_builtin(source: &str, builtin: &'static MetaInfo) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let stream = lexer.tokenize().unwrap();
        let chunk =
            Parser::parse(stream, duka_shared::config::DukaParserConfig::default()).unwrap();
        let registrations: &[BuiltinInfo] = &[(MetaScope::Global, builtin.clone())];
        let (data, errors) = ScopeAnalyzer.analyze(&chunk, Default::default());
        let (data, errors2) =
            DukaTypeChecker::check(&TypeAnalyzer(registrations), &chunk, data, None);
        let (_data, errors3) = DukaTypeChecker::check(&TypeChecker, &chunk, data, None);
        errors.chain(errors2).chain(errors3).collect()
    }

    /// A builtin may name a type the language declares, and the name is read
    /// where the scope is.
    #[test]
    fn a_builtin_can_name_a_type_the_scope_declares() {
        const MEMBER: MetaInfo = MetaInfo {
            name: "answer",
            doc: "",
            example: None,
            info: MetaItemInfo::Function {
                returns: ReturnMeta {
                    text: "",
                    tys: &[DocType::Named("Status", &[])],
                    var_arg: false,
                },
                params: &[],
                type_params: &[],
                where_clauses: &[],
                constraints: &[],
            },
            flags: &[],
        };
        static MODULE: MetaInfo = MetaInfo {
            name: "test",
            doc: "",
            example: None,
            info: MetaItemInfo::Module { inner: &[MEMBER] },
            flags: &[],
        };
        let errors = check_with_builtin(
            "type Status = string\nlocal x: int = answer()\nreturn x\n",
            &MODULE,
        );
        assert!(
            errors.iter().any(|e| e.to_string().contains("string")),
            "{errors:?}"
        );
    }

    /// A name applied to arguments is a type function call, the way a type
    /// position reads it.
    #[test]
    fn a_builtin_can_apply_a_type_function_the_scope_declares() {
        const MEMBER: MetaInfo = MetaInfo {
            name: "answer",
            doc: "",
            example: None,
            info: MetaItemInfo::Function {
                returns: ReturnMeta {
                    text: "",
                    tys: &[DocType::Named("Nilable", &[DocType::Base(Type::Bool)])],
                    var_arg: false,
                },
                params: &[],
                type_params: &[],
                where_clauses: &[],
                constraints: &[],
            },
            flags: &[],
        };
        static MODULE: MetaInfo = MetaInfo {
            name: "test",
            doc: "",
            example: None,
            info: MetaItemInfo::Module { inner: &[MEMBER] },
            flags: &[],
        };
        let errors = check_with_builtin(
            "type function Nilable(t)\n    return t?\nend\nlocal x: int = answer()\nreturn x\n",
            &MODULE,
        );
        assert!(
            errors.iter().any(|e| e.to_string().contains("bool")),
            "{errors:?}"
        );
    }

    fn check(source: &str) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let stream = lexer.tokenize().unwrap();
        let chunk =
            Parser::parse(stream, duka_shared::config::DukaParserConfig::default()).unwrap();
        dbg!(&chunk);
        dbg!(
            ScopeAnalyzer
                .chain(TypeChecker)
                .analyze(&chunk, Default::default())
                .1
                .collect()
        )
    }

    fn is_error(errors: &[DukaSpannedError]) -> bool {
        errors.iter().any(|e| {
            matches!(
                e.kind,
                DukaErrorKind::Semantic(
                    DukaSemanticError::TypeMismatchEqual(..)
                        | DukaSemanticError::TypeMismatchReturn(..)
                        | DukaSemanticError::TypeMismatchArg(..)
                        | DukaSemanticError::TypeParamBoundViolated(..)
                )
            )
        })
    }

    /// A type argument written on a member call is not decoration: it fixes the
    /// variables, and the arguments have to fit what it fixed them to.
    #[test]
    fn explicit_member_type_argument_is_checked() {
        let ok = check(
            "local function push<T>(arr: array<T>, val: T)\nend\nlocal t = { push = push }\nt.push.<int>([1], 2)\nreturn 0\n",
        );
        assert!(!is_error(&ok), "{ok:?}");
        let errors = check(
            "local function push<T>(arr: array<T>, val: T)\nend\nlocal t = { push = push }\nt.push.<string>([1], 13)\nreturn 0\n",
        );
        assert!(is_error(&errors), "{errors:?}");
    }

    /// A generic `for` over an array or table names its variables from what it
    /// iterates: an array yields its elements, a table its value, or its key
    /// and value when two names are written.
    #[test]
    fn generic_for_names_its_variables_from_what_it_iterates() {
        let errors = check(
            r#"
local arr = [1, 2]
for v in arr do
    local i: int = v
end
local tab = { a = 1 }
for v in tab do
    local i: int = v
end
for k, v in tab do
    local ks: string = k
    local i: int = v
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    #[test]
    fn structural_union_narrows_by_element_comparison() {
        let errors = check(
            r#"
local function f(a: ["true", int] | ["false", bool])
    if a[0] == "true" then
        local n: int = a[1]
    end
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    #[test]
    fn structural_union_narrows_by_element_comparison_else() {
        let errors = check(
            r#"
local function f(a: ["true", int] | ["false", bool])
    if a[0] == "true" then
        local n: int = a[1]
    else
        local b: bool = a[1]
    end
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    #[test]
    fn wide_scalar_element_narrows_only_where_possible() {
        let narrowed = check(
            r#"
local function f(u: [int, int] | [string, bool])
    if u[0] == 1 then
        local n: int = u[1]
    end
end
return 0
"#,
        );
        assert!(!is_error(&narrowed), "{narrowed:?}");

        let conservative = check(
            r#"
local function f(u: [int, int] | [string, bool])
    if u[0] == 1 then
    else
        local b: bool = u[1]
    end
end
return 0
"#,
        );
        assert!(is_error(&conservative), "{conservative:?}");
    }

    #[test]
    fn while_condition_narrows_structural_union() {
        let errors = check(
            r#"
local function f(a: ["true", int] | ["false", bool])
    while a[0] == "true" do
        local n: int = a[1]
    end
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    #[test]
    fn member_path_narrows_a_record_field() {
        let src = "local function f(cfg: { host: \"a\" | \"b\", port: int }) ";
        let ok = check(&format!(
            "{src} if cfg.host == \"a\" then local s: \"a\" = cfg.host end \
                   if cfg.host ~= \"a\" then local t: \"b\" = cfg.host end \
                   end return 0"
        ));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!(
            "{src} if cfg.host == \"a\" then local s: \"b\" = cfg.host end end return 0"
        ));
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn member_path_rebuilds_through_several_levels() {
        let src = "local function f(cfg: { meta: { name: string? } }) ";
        // `string!` is not nilable, so the read passes only where the check narrowed
        let ok = check(&format!(
            "{src} if cfg.meta.name ~= nil then local s: string! = cfg.meta.name end \
                   end return 0"
        ));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!(
            "{src} local s: string! = cfg.meta.name end return 0"
        ));
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn member_path_walks_a_tuple_slot_into_a_record() {
        let src = "local function f(p: [{ kind: \"x\" | \"y\", n: int }]) ";
        let ok = check(&format!(
            "{src} if p[0].kind == \"x\" then \
                   local k: \"x\" = p[0].kind local n: int = p[0].n end \
                   end return 0"
        ));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!(
            "{src} if p[0].kind == \"x\" then local k: \"y\" = p[0].kind end end return 0"
        ));
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn a_computed_index_stops_the_walk() {
        // `u[i]` names no literal key, so nothing is read and the union stays
        let errors = check(
            r#"
local function f(u: ["true", int] | ["false", bool])
    local i = 0
    if u[i] == "true" then
        local n: int = u[1]
    end
end
return 0
"#,
        );
        assert!(is_error(&errors), "{errors:?}");
    }

    #[test]
    fn a_shared_element_is_not_narrowed_from_one_row() {
        // every row has one element type: what one row's flag checks says
        // nothing about the rows as a whole
        let errors = check(
            r#"
local function f(rows: array<{ flag: "y" | "n" }>)
    if rows[0].flag == "y" then
        local k: "y" = rows[0].flag
    end
end
return 0
"#,
        );
        assert!(is_error(&errors), "{errors:?}");
    }

    #[test]
    fn a_nil_check_on_a_local_removes_the_nil() {
        // a tuple answers as itself at runtime -- `__eq` is looked up on
        // tables and userdata only -- so `~=` with `nil` is read exactly
        let errors = check(
            r#"
local bcd: (['false', int] | ['true']) = ["false" |> as_const, 1]
if bcd ~= nil then
    local q: (['false', int] | ['true'])! = bcd
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    #[test]
    fn a_nil_check_joined_with_an_element_comparison_both_hold() {
        // both write one fact about the name and the block sees both
        for cond in [
            "bcd ~= nil and bcd[0] == 'false'",
            "bcd[0] == 'false' and bcd ~= nil",
        ] {
            let errors = check(&format!(
                "local bcd: (['false', int] | ['true']) = [\"false\" |> as_const, 1]\n\
                 if {cond} then\n\
                     local q: (['false', int] | ['true'])! = bcd\n\
                 end\n\
                 return 0\n"
            ));
            assert!(!is_error(&errors), "{cond}: {errors:?}");
        }
    }

    #[test]
    fn an_optional_index_comparison_narrows_away_the_nil() {
        // `x?[0]` answered with a value, so the base was there to read
        let errors = check(
            r#"
local bcd: (['false', int] | ['true']) = ["false" |> as_const, 1]
if bcd?[0] == 'false' then
    local q: (['false', int] | ['true'])! = bcd
    local e: ['false', int] = bcd
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    #[test]
    fn a_wide_join_narrows_through_while() {
        let errors = check(
            r#"
local bcd: (['false', int] | ['true']) = ["false" |> as_const, 1]
while bcd ~= nil and bcd[0] == 'false' do
    local q: (['false', int] | ['true'])! = bcd
    break
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    #[test]
    fn a_nil_comparison_with_a_function_narrows_the_nil_branch() {
        // `set_metatable` takes a table, so a function can carry no `__eq` --
        // `== nil` is read exactly and the branch sees the nil
        let errors = check(
            r#"
local f: fn(int) -> int = nil
if f == nil then
    local h: nil = f
end
return 0
"#,
        );
        assert!(!is_error(&errors), "{errors:?}");
    }

    /// A stage that declares what a builtin says about its own call, so that
    /// a test can write one without a runtime to provide it.
    struct WithRules(BuiltinRules);

    impl DukaAnalyzer for WithRules {
        type InputType = DukaChunk;
        type InputData = AnalyzerData;
        type OutputData = AnalyzerData;

        fn analyze(
            &self,
            _chunk: &DukaChunk,
            data: AnalyzerData,
        ) -> (AnalyzerData, impl Iterator<Item = DukaSpannedError>) {
            let (config, mut analysis) = data;
            analysis.builtin_rules.insert("f".into(), self.0.clone());
            ((config, analysis), std::iter::empty())
        }
    }

    /// The errors a source produces when a builtin called `f` says what it says
    /// about its own call.
    fn check_with_rules(source: &str, rules: BuiltinRules) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let stream = lexer.tokenize().unwrap();
        let chunk =
            Parser::parse(stream, duka_shared::config::DukaParserConfig::default()).unwrap();
        ScopeAnalyzer
            .chain(WithRules(rules))
            .chain(TypeChecker)
            .analyze(&chunk, Default::default())
            .1
            .collect()
    }

    /// A `where` bound is an obligation the call has to meet, and it is met
    /// with the types the arguments turned out to have.
    #[test]
    fn a_where_bound_is_checked_at_the_call() {
        let rules = BuiltinRules {
            where_clauses: vec![WhereMeta::Bound {
                var: "T",
                ty: DocType::Base(Type::Int),
            }]
            .into(),
            constraints: vec![].into(),
        };
        let errors = check_with_rules(
            "function f<T>(x: T): T\n    return x\nend\nlocal y = f(\"s\")\n",
            rules.clone(),
        );
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::WhereBoundViolated(..))
            )),
            "{errors:?}"
        );
        // and the bound that holds is not a failure
        let errors = check_with_rules(
            "function f<T>(x: T): T\n    return x\nend\nlocal y = f(1)\n",
            rules,
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// A concept is read for its truth, and a builtin names one and gives it
    /// its arguments rather than writing the expression out.
    #[test]
    fn a_where_concept_is_discharged_at_the_call() {
        static ARGS: &[DocType] = &[DocType::Base(Type::Int)];
        let rules = BuiltinRules {
            where_clauses: vec![WhereMeta::Concept {
                name: "IsIterable",
                args: ARGS,
            }]
            .into(),
            constraints: vec![].into(),
        };
        let errors = check_with_rules(
            "function f<T>(x: T): T\n    return x\nend\nlocal y = f(1)\n",
            rules,
        );
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::WhereConceptFailed)
            )),
            "{errors:?}"
        );
    }

    /// The rendered type of every type alias the source declares, which is the
    /// only way to see what a type annotation actually solved to.
    fn alias_types(source: &str) -> std::collections::HashMap<String, String> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let chunk = Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .unwrap();
        let pipeline = crate::analyzer::ScopeAnalyzer
            .chain(crate::analyzer::BasicAnalyzer)
            .chain(crate::analyzer::TypeEval)
            .chain(TypeChecker);
        let ((_, analysis), errors) = pipeline.analyze(&chunk, Default::default());
        let errors: Vec<_> = errors.collect();
        assert!(errors.is_empty(), "{:?}", errors);
        let mut out = std::collections::HashMap::new();
        for scope in analysis.symbols.scopes() {
            for (name, syms) in &scope.symbols {
                if let Some(last) = syms.last() {
                    out.insert(
                        name.to_string(),
                        last.ty_display.as_deref().unwrap_or("").to_owned(),
                    );
                }
            }
        }
        out
    }

    #[test]
    fn returns_of_different_lengths_line_up_by_position() {
        let types = alias_types(
            "function C<T>(a: T, ...)\n\
             \x20   if a == nil then return a end\n\
             \x20   return a, 1, 2\n\
             end\n",
        );
        assert_eq!(
            types.get("C").map(String::as_str),
            Some("function(T?, ...) -> (T?, int, int, ...)")
        );
    }

    #[test]
    fn a_return_that_always_fills_the_same_slots_is_not_var_arg() {
        let types = alias_types("function B<T>(a: T, ...)\n    return a, 1\nend\n");
        assert_eq!(
            types.get("B").map(String::as_str),
            Some("function(T?, ...) -> (T?, int)")
        );
    }

    #[test]
    fn a_varying_tail_needs_the_rest_parameter_to_exist() {
        let with = alias_types(
            "function D(a, ...)\n\
             \x20   if a == nil then return end\n\
             \x20   return a, 1, 2\n\
             end\n",
        );
        assert_eq!(
            with.get("D").map(String::as_str),
            Some("function(any, ...) -> (any, int, int, ...)")
        );
        let without = alias_types(
            "function E(a)\n\
             \x20   if a == nil then return end\n\
             \x20   return a, 1\n\
             end\n",
        );
        assert_eq!(
            without.get("E").map(String::as_str),
            Some("function(any) -> (any, int)")
        );
    }

    #[test]
    fn two_returns_of_one_value_do_not_become_two() {
        let types = alias_types(
            "function F(a)\n\
             \x20   if a == nil then return 1 end\n\
             \x20   return 2\n\
             end\n",
        );
        assert_eq!(
            types.get("F").map(String::as_str),
            Some("function(any) -> int")
        );
    }

    #[test]
    fn a_return_protocol_says_what_the_slots_are() {
        let types = alias_types(
            "@returns(result) function f(a: int)\n\
             \x20   return 1, a\n\
             end\n\
             @returns(exit) function g()\n\
             \x20   return\n\
             end\n\
             function h() return 1 end\n",
        );
        assert_eq!(
            types.get("f").map(String::as_str),
            Some("function(int?) -> (bool, ...)"),
            "the first slot says whether it succeeded and the rest are its own values"
        );
        assert_eq!(
            types.get("g").map(String::as_str),
            Some("function()"),
            "nothing comes back, so there is no question of what it returned"
        );
        assert_eq!(
            types.get("h").map(String::as_str),
            Some("function() -> int"),
            "a declaration that said nothing keeps what it inferred"
        );
    }

    #[test]
    fn written_returns_win_over_the_return_protocol() {
        let types = alias_types(
            "@returns(result) function f(a: int): (int, string)\n\
             \x20   return 1, \"s\"\n\
             end\n",
        );
        assert_eq!(
            types.get("f").map(String::as_str),
            Some("function(int?) -> (int, string)"),
            "what is written out says what the slots are"
        );
    }

    #[test]
    fn a_keywordish_declaration_carries_that_on_the_symbol() {
        let lexer = LexerWithMacro::new(
            Cursor::new("@keywordish function f() end\n"),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let chunk = Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .unwrap();
        let ((_, analysis), _) = ScopeAnalyzer.analyze(&chunk, Default::default());
        let declared = analysis.symbols.lookup_named("f").expect("f is declared");
        assert!(declared.attributes.has_keywordish());
        let plain = alias_types("function g() end\n");
        assert_eq!(plain.get("g").map(String::as_str), Some("function()"));
    }

    #[test]
    fn a_declaration_is_read_off_the_signature_it_wrote() {
        let types = alias_types(
            "@declare function strlen(s: string): int\n\
             @declare @const ENDIAN: string\n\
             @declare global host_flag: bool\n",
        );
        assert_eq!(
            types.get("strlen").map(String::as_str),
            Some("function(string?) -> int"),
            "there is no body to infer from, so the annotation is the whole of it"
        );
        assert_eq!(types.get("ENDIAN").map(String::as_str), Some("string?"));
        assert_eq!(types.get("host_flag").map(String::as_str), Some("bool?"));
    }

    #[test]
    fn a_declaration_with_no_annotation_says_nothing_rather_than_saying_nil() {
        let types = alias_types("@declare global whatever\n@declare function f()\n");
        assert_eq!(
            types.get("whatever").map(String::as_str),
            Some("any"),
            "a value that was never written is not a nil"
        );
        assert_eq!(
            types.get("f").map(String::as_str),
            Some("function()"),
            "and a return that was never annotated is no return at all"
        );
    }

    #[test]
    fn a_declaration_still_reads_the_other_attributes_written_with_it() {
        let types = alias_types("@declare @returns(result) function f(a: int)\n");
        assert_eq!(
            types.get("f").map(String::as_str),
            Some("function(int?) -> (bool, ...)")
        );
    }

    #[test]
    fn a_declaration_with_something_to_build_is_refused() {
        let built = |kind| {
            check(kind).into_iter().any(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::DeclaredIsNotBuilt(..))
                )
            })
        };
        assert!(built("@declare function f() = 1\n"));
        assert!(built("@declare local x: int = 1\n"));
        assert!(
            !built("@declare function f(): int\n"),
            "a declaration has nothing to build, which is the whole of it"
        );
    }

    #[test]
    fn a_recursive_tail_is_the_same_recursive_type_however_often_it_is_read() {
        let types = alias_types(
            "type function LList(T) = [T, LList(T)?]\n\
             type A1 = LList(int)[1]\n\
             type A2 = LList(int)[1][1]\n\
             type A3 = LList(int)[1][1][1]\n",
        );
        let tail = types.get("A1").expect("A1");
        assert_eq!(tail, "rec [int, LList?]?");
        assert_eq!(types.get("A2").map(String::as_str), Some(tail.as_str()));
        assert_eq!(types.get("A3").map(String::as_str), Some(tail.as_str()));
    }

    #[test]
    fn reading_a_recursive_tail_does_not_leak_the_recursion_placeholder() {
        let types =
            alias_types("type function LList(T) = [T, LList(T)?]\ntype A2 = LList(int)[1][1]\n");
        let tail = types.get("A2").expect("A2");
        assert!(tail.starts_with("rec "), "{tail}");
        assert!(!tail.contains("__rec_"), "{tail}");
    }

    #[test]
    fn each_instantiation_of_a_recursive_type_function_keeps_its_own_tail() {
        let types = alias_types(
            "type function LList(T) = [T, LList(T)?]\n\
             type A = LList(int)[1]\n\
             type B = LList(string)[1]\n",
        );
        assert_eq!(
            types.get("A").map(String::as_str),
            Some("rec [int, LList?]?")
        );
        assert_eq!(
            types.get("B").map(String::as_str),
            Some("rec [string, LList?]?")
        );
    }

    #[test]
    fn a_record_reads_as_a_table_in_type_context() {
        let types = alias_types(
            "type D1 = { a: [int, string] }[\"a\"][1]\ntype D2 = { a: [int, string] }.a[1]\n",
        );
        assert_eq!(types.get("D1").map(String::as_str), Some("string"));
        assert_eq!(types.get("D2").map(String::as_str), Some("string"));
    }

    #[test]
    fn a_rest_pack_spreads_into_the_tuple() {
        let types = alias_types(
            "type function FFFF(...ts) = [int, ...ts]\n\
             type TEST = FFFF(int, false)\n\
             type TEST2 = FFFF(int, false, string)\n",
        );
        assert_eq!(
            types.get("TEST").map(String::as_str),
            Some("[int, int, false]")
        );
        assert_eq!(
            types.get("TEST2").map(String::as_str),
            Some("[int, int, false, string]")
        );
    }

    fn parse_err(source: &str) -> bool {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .is_err()
    }

    use duka_shared::errors::{DukaErrorKind, DukaSemanticError, DukaSpannedError};

    #[test]
    fn accepts_int_for_num() {
        let errors = check("local n: num = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_int_for_float() {
        let errors = check("local n: float = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_float_for_int() {
        let errors = check("local n: int = 1.5");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_float_for_num() {
        let errors = check("local n: num = 1.5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_string_for_int() {
        let errors = check("local n: int = \"hi\"");
        assert!(is_error(&errors), "expected a type error, got {:?}", errors);
    }

    #[test]
    fn allows_any_to_silent_unknown() {
        let errors = check("local n: int = some_unknown_call()");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_wrong_return_type() {
        let errors = check("function f(): int return \"no\" end");
        assert!(
            is_error(&errors),
            "expected return type error, got {:?}",
            errors
        );
    }

    #[test]
    fn accepts_correct_return_type() {
        let errors = check("function f(): int return 42 end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_float_literal_bitand() {
        let errors = check("local x = 5.5 & 1");
        assert!(
            is_error(&errors),
            "expected float bitand error, got {:?}",
            errors
        );
    }

    #[test]
    fn allows_float_operand_unknown_meta() {
        let errors = check("local x = a & 1.5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_int_bitand() {
        let errors = check("local n: int = 5 & 3");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn propagates_param_type_to_body() {
        let errors = check("function f(a: int) a = \"hi\" end");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn propagates_local_type_to_reassign() {
        let errors = check("local n: num = 1 n = \"hi\"");
        assert!(
            is_error(&errors),
            "expected assignment type error {:?}",
            errors
        );
    }

    #[test]
    fn infers_local_type_from_literal() {
        let errors = check("local n = 1 n = \"hi\"");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_correct_reassign() {
        let errors = check("local n: num = 1 n = 2.5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn infers_constant_type() {
        let errors = check("@const local N = 3");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_union_member() {
        let errors = check("local x: int | nil = 5");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_union_non_member() {
        let errors = check("local x: int | nil = \"hi\"");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_union_subtype_member() {
        let errors = check("local x: float | nil = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_empty_type_annotation() {
        assert!(
            parse_err("local a: = 1"),
            "`local a: = 1` must fail to parse"
        );
    }

    #[test]
    fn rejects_trailing_union_member() {
        assert!(
            parse_err("local a: int | = 1"),
            "`int | = 1` must fail to parse"
        );
    }

    #[test]
    fn accepts_fn_signature_annotation() {
        let errors = check("local cb: fn(int, string) -> bool = function(a, b) return true end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_fn_vararg_annotation() {
        let errors = check("local cb: fn(int, ...) -> int = function(a, ...) return 1 end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_fn_empty_signature() {
        let errors = check("local cb: fn() = function() end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_fn_missing_param_type() {
        assert!(parse_err("local cb: fn(int,) -> int = 1"));
    }

    #[test]
    fn rejects_fn_bad_param_syntax() {
        assert!(parse_err("local x: fn(int -> int) = 5"));
    }

    #[test]
    fn accepts_fn_multi_return() {
        let errors = check("local cb: fn(int) -> (int, string) = function(a) return a, \"x\" end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn accepts_fn_vararg_return_tuple() {
        let errors =
            check("local cb: fn(int, ...) -> (int, ...) = function(a, ...) return 1, ... end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn expands_multi_return_as_call_args() {
        let errors =
            check("function pair() return 1, \"s\" end function take(a, b, c) end take(0, pair())");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_too_few_call_args() {
        let errors = check("function f(a, b) end f(1)");
        assert!(is_error(&errors), "expected arity error {:?}", errors);
    }

    #[test]
    fn deep_member_assign_refines_reads() {
        let errors = check("local a = {b = {c = {}}} a.b.c.d = 1 local s: string = a.b.c.d");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn tuple_literal_index_types() {
        let errors = check("local t = {1, \"s\"} local n: int = t[1]");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn accepts_tuple_annotation_with_rest_tail() {
        let errors =
            check("local a: [int, string, ...] = [1, \"s\", true] local b: [int, ...] = [2]");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rest_tuple_rejects_mismatched_prefix() {
        let errors = check("local a: [int, string, ...] = [1, 2]");
        assert!(is_error(&errors), "expected member error {:?}", errors);
    }

    #[test]
    fn fixed_tuple_annotation_rejects_rest_array() {
        let errors = check("local a: [int, int] = [1, 2, ...]");
        assert!(is_error(&errors), "expected rest mismatch {:?}", errors);
    }

    #[test]
    fn index_then_field_keeps_type() {
        let errors = check("local t = {{x = 1}} local s: string = t[0].x");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn record_literal_key_index_keeps_type() {
        let errors = check("local M = {} M.n = 1 local s: string = M[\"n\"]");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn object_member_assign_must_match() {
        let errors = check("object A\n  n: int\nend\nlocal a = A.new()\na.n = \"no\"");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn object_member_assign_accepts_declared() {
        let errors = check("object A\n  n: int\nend\nlocal a = A.new()\na.n = 1");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn record_field_reassign_must_match() {
        let errors = check("local M = {} M.n = 1 M.n = \"no\"");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn nil_init_keeps_annotated_type() {
        let errors = check("local v: int = nil v = \"no\"");
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn rejects_extra_call_args() {
        // an argument with no parameter to land in used to be dropped at
        // runtime, which reads as a silent no-op at the call site
        let errors = check("function f(a) end f(1, 2, 3)");
        assert!(is_error(&errors), "expected arity error {:?}", errors);
    }

    #[test]
    fn accepts_vararg_param_call_args() {
        let errors = check("function f(a, ...) end f(1, 2, 3)");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_missing_fixed_args_before_vararg() {
        let errors = check("function f(a, b, ...) end f(1)");
        assert!(is_error(&errors), "expected arity error {:?}", errors);
    }

    #[test]
    fn skips_arity_for_open_vararg_call() {
        let errors = check("function g(...): ... return ... end function f(a, b) end f(g())");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn expands_multi_return_in_table_and_array() {
        let errors =
            check("function pair() return 1, \"s\" end local a = {pair()} local b = {pair(), 3}");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn calls_result_of_call() {
        let errors = check(
            "function inner(): int return 1 end function outer(): fn() -> int return inner end local n: int = (outer())()",
        );
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn nilable_annotation_keeps_signature() {
        let errors = check("local x: fn(int) = function(a) end return x()");
        assert!(is_error(&errors), "expected arity error {:?}", errors);
    }

    #[test]
    fn rejects_wrong_type_from_collected_multi_return() {
        let errors = check(
            "function pair() return 1, \"s\" end function g() return pair() end local a, b: fn() = g()",
        );
        assert!(is_error(&errors), "expected type error {:?}", errors);
    }

    #[test]
    fn accepts_fn_bare_vararg_return() {
        let errors = check("local cb: fn(int) -> ... = function(a) return ... end");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn rejects_fn_bad_multi_return_syntax() {
        assert!(parse_err("local cb: fn() -> (int,) = 1"));
    }

    #[test]
    fn rejects_fn_vararg_return_mid_list() {
        assert!(parse_err("local cb: fn() -> (int, ..., string) = 1"));
    }

    fn check_with(source: &str, nonnilable: bool) -> Vec<DukaSpannedError> {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let stream = lexer.tokenize().unwrap();
        let chunk = Parser::parse(
            stream,
            duka_shared::config::DukaParserConfig {
                default_nonnilable: nonnilable,
                ..Default::default()
            },
        )
        .unwrap();
        dbg!(
            ScopeAnalyzer
                .chain(TypeChecker)
                .analyze(
                    &chunk,
                    duka_shared::config::DukaAnalyzerConfig {
                        default_nonnilable: nonnilable,
                        ..Default::default()
                    },
                )
                .1
                .collect()
        )
    }

    #[test]
    fn bang_strips_nil_from_atom() {
        let errors = check_with("local a: int! = nil", false);
        assert!(is_error(&errors), "{:?}", errors);
        let errors = check_with("local a: int! = 5", false);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn union_bang_keeps_other_nil() {
        let errors = check_with("local a: int | string! = nil", false);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn paren_group_bang_strips_whole_union() {
        let errors = check_with("local a: (int | string)! = nil", false);
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn question_suffix_adds_nil() {
        let errors = check_with("local a: int? = nil", false);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_rejects_nil() {
        let errors = check_with("local a: int = nil", true);
        assert!(is_error(&errors), "{:?}", errors);
        let errors = check_with("local a: int = 5", true);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_accepts_question() {
        let errors = check_with("local a: int? = nil", true);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_accepts_union_nil() {
        let errors = check_with("local a: int | nil = nil", true);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn default_nonnilable_union_without_nil_rejects() {
        let errors = check_with("local a: int | string = nil", true);
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn an_inferred_local_takes_nil_when_not_strict() {
        let src = r#"
local x = 1
x = nil
return x
"#;
        let errors = check_with(src, false);
        assert!(!is_error(&errors), "{:?}", errors);
        let errors = check_with(src, true);
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn a_relaxed_local_reads_the_value_it_started_with() {
        let errors = check_with(
            r#"
local function takes_int(v: int!): int
    return v
end
local x = 1
return takes_int(x)
"#,
            false,
        );
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn a_parameter_annotation_follows_the_config() {
        let src = r#"
local function takes_int(v: int!): int
    return v
end
local function f(v: int): int
    return takes_int(v)
end
return 0
"#;
        let errors = check_with(src, false);
        assert!(is_error(&errors), "{:?}", errors);
        let errors = check_with(src, true);
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn object_base_resolves() {
        let errors = check("object A\nend\nobject B : A\nend");
        assert!(
            !errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(
                    DukaSemanticError::UnknownBase(..) | DukaSemanticError::CircularExtends(..)
                )
            )),
            "{:?}",
            errors
        );
    }

    #[test]
    fn generic_param_is_not_any() {
        let ok = check("function id<T>(x: T): T return x end local a: int = id(1)");
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check("function id<T>(x: T): T return x end local a: string = id(1)");
        assert!(is_error(&bad), "T must not degrade to any: {:?}", bad);
    }

    #[test]
    fn const_type_param_restores_the_literal() {
        let src = "function g<const C>(x: C!) return x end";
        let ok = check(&format!("{src} local b: 1 = g(1)"));
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check(&format!("{src} local b: 1 = g(2)"));
        let mismatch = bad
            .iter()
            .find_map(|e| match &e.kind {
                DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(ann, actual)) => {
                    Some((ann.clone(), actual.clone()))
                }
                _ => None,
            })
            .expect("g(2) against an annotation of 1 is a mismatch");
        assert_eq!(mismatch, ("1".to_owned(), "2".to_owned()));
    }

    #[test]
    fn const_type_param_sees_through_a_nilable_annotation() {
        // `x: C` arrives as `C?`, so the return is `1?`, not the widened `int?`
        let bad = check("function g<const C>(x: C) return x end local b: 1 = g(1)");
        let mismatch = bad
            .iter()
            .find_map(|e| match &e.kind {
                DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(ann, actual)) => {
                    Some((ann.clone(), actual.clone()))
                }
                _ => None,
            })
            .expect("x: C returns C?, which a bare 1 does not accept");
        assert_eq!(mismatch, ("1".to_owned(), "1?".to_owned()));
    }

    #[test]
    fn var_arg_collect_binds_the_pack_to_the_call_tail() {
        let src = "function b<...Ts: [int, string]>(...: Ts) return 1 end";
        let ok = check(&format!("{src} local x = b(1, \"s\")"));
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check(&format!("{src} local x = b(1, 2)"));
        assert!(
            is_error(&bad),
            "the tail is collected, so the bound holds: {:?}",
            bad
        );
        // a multi-return call in the final position expands
        let ok = check(
            "function a() return 1, \"s\" end \
             function b<...Ts: [int, string]>(...: Ts) return 1 end \
             local x = b(a())",
        );
        assert!(!is_error(&ok), "{:?}", ok);
    }

    #[test]
    fn var_arg_collect_target_must_be_a_declared_pack() {
        let bad = check("function d(...: int) return 1 end");
        assert!(
            bad.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(_))
            )),
            "`...: int` names no pack variable: {:?}",
            bad
        );
    }

    #[test]
    fn var_arg_pack_bound_can_be_written_as_where() {
        let src = "function b<...Ts>(...: Ts) where Ts: [int, string] return 1 end";
        let ok = check(&format!("{src} local x = b(1, \"s\")"));
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check(&format!("{src} local x = b(1, 2)"));
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn return_pack_reads_the_solved_tail() {
        let src = "function pipe<...Rs>(f: fn() -> ...Rs): ...Rs return f() end \
                   function source(): (int, string) return 1, \"s\" end ";
        let ok = check(&format!(
            "{src} local a, b = pipe(source) \
                                 local x: int = a local y: string = b"
        ));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!(
            "{src} local a, b = pipe(source) local x: string = a"
        ));
        assert!(
            is_error(&bad),
            "the pack solves to [int, string]: {:?}",
            bad
        );
    }

    #[test]
    fn return_pack_keeps_declared_slots_in_front_of_the_tail() {
        let src = "function wrap<...Rs>(f: fn() -> ...Rs): (bool, ...Rs) \
                   return true, f() end \
                   function src(): (int, string) return 1, \"s\" end ";
        let ok = check(&format!(
            "{src} local ok, v1, v2 = wrap(src) \
                                 local c: int = v1 local d: string = v2"
        ));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!(
            "{src} local ok, v1, v2 = wrap(src) local x: string = v1"
        ));
        assert!(
            is_error(&bad),
            "the slot before the pack stays int: {:?}",
            bad
        );
    }

    #[test]
    fn written_returns_win_over_the_result_protocol() {
        let src = "@returns(result) function f(): (bool, int) return true, 1 end";
        let ok = check(&format!("{src} local n, m = f() local x: int = m"));
        assert!(ok.is_empty(), "{:?}", ok);
        // `m` would be any if the protocol had forced the tail open
        let bad = check(&format!("{src} local n, m = f() local x: string = m"));
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn return_pack_target_must_be_a_declared_pack() {
        let bad = check("function g(): ...Rs return 1 end");
        assert!(
            bad.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(_))
            )),
            "`...Rs` names no pack variable: {:?}",
            bad
        );
    }

    #[test]
    fn return_pack_binds_through_a_bound_position() {
        let src = "function call2<T: fn() -> ...Rs, ...Rs>(f: T): ...Rs return f() end \
                   function src2(): (int, string) return 1, \"s\" end ";
        let ok = check(&format!(
            "{src} local a, b = call2(src2) \
                                 local x: int = a local y: string = b"
        ));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!(
            "{src} local a, b = call2(src2) local x: string = a"
        ));
        assert!(is_error(&bad), "the bound carries the pack: {:?}", bad);
    }

    #[test]
    fn return_pack_bound_can_be_written_as_where() {
        let src = "function call3<T: fn() -> ...Rs, ...Rs>(f: T): ...Rs \
                   where Rs: [int, string] return f() end \
                   function good(): (int, string) return 1, \"s\" end \
                   function off(): (int, int) return 1, 2 end ";
        let ok = check(&format!("{src} local a, b = call3(good)"));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!("{src} local a, b = call3(off)"));
        assert!(
            is_error(&bad),
            "the where tightens the solved pack: {:?}",
            bad
        );
    }

    #[test]
    fn generic_return_annotation_is_substituted() {
        let errors = check("function id<T>(x: T): T return x end local a: string = id(1)");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_bound_is_enforced_after_inference() {
        let errors = check("function bnd<T: int>(x: T): T return x end local a = bnd(\"s\")");
        assert!(is_error(&errors), "expected bound error {:?}", errors);
    }

    #[test]
    fn a_bound_violation_is_reported_once() {
        let errors = check("function bnd<T: int>(x: T): T return x end local a = bnd(\"s\")");
        let count = errors
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeParamBoundViolated(..))
                )
            })
            .count();
        assert_eq!(count, 1, "{:?}", errors);
    }

    #[test]
    fn a_generic_method_solves_from_its_arguments() {
        let src = "object Box\n    function :id<U>(x: U): U\n        return x\n    end\nend\nlocal b = Box.new()\n";
        let ok = check(&format!("{src}local n: int = b:id(1)"));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!("{src}local s: string = b:id(1)"));
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn a_colon_method_takes_explicit_type_arguments() {
        let src = "object Box\n    function :id<U>(x: U): U\n        return x\n    end\nend\nlocal b = Box.new()\n";
        let ok = check(&format!("{src}local n: string = b:id.<string>(\"x\")"));
        assert!(ok.is_empty(), "{:?}", ok);
        let bad = check(&format!("{src}local n = b:id.<string>(1)"));
        assert!(is_error(&bad), "{:?}", bad);
        let arity = check(&format!("{src}local n = b:id.<string, int>(\"x\")"));
        assert!(
            arity.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::TypeArgArityMismatch(..))
            )),
            "{arity:?}"
        );
    }

    #[test]
    fn a_generic_method_enforces_its_bound() {
        let src = "object Box\n    function :bnd<U: int>(x: U): U\n        return x\n    end\nend\nlocal b = Box.new()\n";
        let bad = check(&format!("{src}local n = b:bnd(\"s\")"));
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn a_bound_violation_names_the_type_argument() {
        let errors = check("function bnd<T: int>(x: T): T return x end local a = bnd(\"s\")");
        let message = errors
            .iter()
            .find_map(|e| match &e.kind {
                DukaErrorKind::Semantic(DukaSemanticError::TypeParamBoundViolated(
                    name,
                    bound,
                    candidate,
                )) => Some(format!("{name} {bound} {candidate}")),
                _ => None,
            })
            .expect("a bound violation");
        // `T: int` is a bound, so the bound named in the error is `int`: a number or a
        // nil is not a number, and saying so is the point of the error.
        assert_eq!(message, "T int string");
        let help = errors
            .iter()
            .find(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeParamBoundViolated(..))
                )
            })
            .map(|e| e.kind.get_help())
            .unwrap_or_default();
        assert!(help.contains("'T'"), "{help}");
        // The bound as the programmer wrote it: `T: int` names `int`, and a `?`
        // here would quietly make every bound satisfiable by nil.
        assert!(help.contains("int"), "{help}");
    }

    #[test]
    fn generic_type_function_param_participates() {
        let src = "type function Boxed(t) return { value = t } end function unbox<T>(x: Boxed(T)): T return x.value end";
        let ok = check(&format!("{src} local a: int = unbox({{ value = 1 }})"));
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check(&format!("{src} local a: string = unbox({{ value = 1 }})"));
        assert!(
            is_error(&bad),
            "type function must see the real T: {:?}",
            bad
        );
    }

    #[test]
    fn generic_param_default_is_used_when_unresolved() {
        let src = "function f<T = int>(): T return 1 end";
        let ok = check(&format!("{src} local a: int = f()"));
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check(&format!("{src} local a: string = f()"));
        assert!(is_error(&bad), "the default must decide T: {:?}", bad);
    }

    #[test]
    fn generic_explicit_type_argument_beats_default() {
        let errors =
            check("function f<T = int>(): T return \"s\" end local a: string = f.<string>()");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_explicit_type_argument_checks_the_argument() {
        let ok = check("function id<T>(x: T): T return x end local a: int = id.<int>(1)");
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check("function id<T>(x: T): T return x end local a: int = id.<int>(\"s\")");
        assert!(
            is_error(&bad),
            "explicit type argument must be checked: {:?}",
            bad
        );
    }

    #[test]
    fn generic_shape_mismatch_is_reported_once() {
        let errors = check("function f<T>(x: array<T>) end f(1)");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_opaque_argument_is_not_checked() {
        let errors = check("function f<T>(x: T) end f(unknown())");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_argument_type_must_match_inference() {
        let errors = check("function f<T>(x: array<T>, y: T) end f([1], 2)");
        assert!(!is_error(&errors), "{:?}", errors);
        let bad = check("function f<T>(x: T, y: T) end f(1, \"s\")");
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn nested_generic_annotation_parses() {
        let ok = check("local a: array<array<int>> = { { 1 } }");
        assert!(!is_error(&ok), "{:?}", ok);
        let bad = check("local a: array<array<int>> = { { \"s\" } }");
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn deeply_nested_generic_annotation_parses() {
        let ok = check("local a: array<array<array<int>>> = { { { 1 } } }");
        assert!(!is_error(&ok), "{:?}", ok);
    }

    #[test]
    fn generic_default_may_use_another_param() {
        let errors =
            check("function f<T, U = array<T>>(): U return { 1 } end local a: array<int> = f()");
        assert!(!is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn generic_bound_and_default_together() {
        let errors = check("function f<T: num = int>(x: T): T return x end local a: int = f(1)");
        assert!(!is_error(&errors), "{:?}", errors);
        let bad = check("function f<T: num = int>(x: T): T return x end local a: int = f(\"s\")");
        assert!(is_error(&bad), "{:?}", bad);
    }

    #[test]
    fn generic_default_and_nested_angle_do_not_panic() {
        for src in [
            "function f<T = int>(): T return 1 end local a: int = f()",
            "local a: array<array<int>> = 1",
        ] {
            let _ = check(src);
        }
    }

    #[test]
    fn generic_param_default_is_not_unresolved_yet() {
        // `T = int` is not parsed yet, the default path lands with the solver
        let errors = check("function f<T>(x: T): T return x end local a: string = f(1)");
        assert!(is_error(&errors), "{:?}", errors);
    }

    #[test]
    fn object_circular_base() {
        let errors = check("object A : B\nend\nobject B : A\nend");
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::CircularExtends(..))
            )),
            "{:?}",
            errors
        );
    }

    fn analyze(source: &str) -> (Vec<DukaSpannedError>, crate::analyzer::ScopeAnalysis) {
        let lexer = LexerWithMacro::new(
            Cursor::new(source),
            SourceName::Virtual("test".into()),
            Default::default(),
        );
        let chunk = Parser::parse(
            lexer.tokenize().unwrap(),
            duka_shared::config::DukaParserConfig::default(),
        )
        .unwrap();
        let sa = ScopeAnalyzer.chain(TypeChecker);
        let (data, errors) = sa.analyze(&chunk, Default::default());
        (errors.collect(), data.1)
    }

    #[test]
    fn object_typed_instance_no_error() {
        let (errors, analysis) = analyze("object A\nend\nlocal a: A = A.new()\nlocal b = a");
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(analysis.objects.len(), 1);
    }

    #[test]
    fn object_unknown_annotation() {
        let (errors, _) = analyze("local a: NoSuch = 1");
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
            )),
            "{:?}",
            errors
        );
    }

    #[test]
    fn a_method_reads_self_through_its_class() {
        let (errors, _) = analyze(
            "object A\n    value: int = 0\n    function :bump(a: int): int\n        return self.value + a\n    end\nend\nlocal a: A = A.new()\nlocal n: int = a:bump(1)",
        );
        assert!(errors.is_empty(), "{:?}", errors);

        // `self.value` is the member's type, not `any`: assigning it to a
        // string is a mismatch rather than a silent pass.
        let (errors, _) = analyze(
            "object A\n    value: int = 0\n    function :bump()\n        local s: string = self.value\n    end\nend",
        );
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(..))
            )),
            "{:?}",
            errors
        );
    }

    #[test]
    fn a_base_class_member_and_method_are_read_on_the_derived_one() {
        let (errors, _) = analyze(
            "object A\n    value: int = 1\n    function :get(): int\n        return self.value\n    end\nend\nobject B extends A\nend\nlocal b: B = B.new()\nlocal s: string = b.value\nlocal t: string = b:get()",
        );
        let mismatches = errors
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(..))
                )
            })
            .count();
        assert_eq!(mismatches, 2, "{:?}", errors);
    }

    #[test]
    fn a_metatable_lends_a_member_through_index() {
        let (errors, _) = analyze(
            "local base = { greet = function(s: string): string return s end }\nlocal t = set_metatable({}, { __index = base })\nlocal ok: string = t.greet(\"x\")\nlocal bad: int = t.greet(\"x\")",
        );
        let mismatches = errors
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(..))
                )
            })
            .count();
        assert_eq!(mismatches, 1, "{:?}", errors);
    }

    #[test]
    fn set_metatable_says_what_get_metatable_reads() {
        let (errors, _) = analyze(
            "local mt = { __index = {} }\nlocal t = set_metatable({}, mt)\nlocal m = get_metatable(t)\nlocal s: string = m",
        );
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(..))
            )),
            "the metatable is a table, not a string: {:?}",
            errors
        );

        // only a table or an object can take one
        let (errors, _) = analyze("local x = set_metatable(1, {})");
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchArg(..))
            )),
            "{:?}",
            errors
        );
    }

    #[test]
    fn a_record_keeps_a_metatable_and_lends_its_members() {
        let (errors, _) = analyze(
            "local base = { name = \"foo\" }\nlocal obj = set_metatable({ value = 1 }, { __index = base })\nlocal s: string = obj.name\nlocal bad: int = obj.name\nlocal off = set_metatable(obj, nil)\nlocal n: int = get_metatable(off)",
        );
        let mismatches = errors
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(..))
                )
            })
            .count();
        // `obj.name` is a string, so the `int` annotation is the one mismatch;
        // detaching leaves `nil`, which a nilable annotation takes
        assert_eq!(mismatches, 1, "{:?}", errors);
    }

    #[test]
    fn a_class_metamethod_types_the_operator() {
        let (errors, _) = analyze(
            "object A\n    function :init(v: int)\n        self.n = v\n    end\n    function __len(): int\n        return self.n\n    end\n    function __add(o: A): int\n        return 1\n    end\nend\nlocal a: A = A.new(1)\nlocal n: int = #a\nlocal bad: string = a + a",
        );
        let mismatches = errors
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(..))
                )
            })
            .count();
        assert_eq!(mismatches, 1, "{:?}", errors);
    }

    #[test]
    fn new_checks_the_arguments_init_takes() {
        let (errors, _) = analyze(
            "object A\n    function :init(v: int)\n        self.v = v\n    end\nend\nlocal a: A = A.new(\"no\")",
        );
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(
                    DukaSemanticError::TypeMismatchArg(..)
                        | DukaSemanticError::TypeMismatchEqual(..)
                )
            )),
            "{:?}",
            errors
        );
    }

    #[test]
    fn self_and_super_are_typed_in_methods() {
        let (errors, _) = analyze(
            "object A\n    value: int = 0\n    function :init(v: int)\n        self.value = v\n    end\nend\nobject B extends A\n    function :init(v: int)\n        super:init(v)\n        local s: string = self.value\n    end\nend\nlocal b: B = B.new(1)",
        );
        let mismatches = errors
            .iter()
            .filter(|e| {
                matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::TypeMismatchEqual(..))
                )
            })
            .count();
        // `self.value` is the inherited `int`: that annotation is the one mistake,
        // and `super:init(v)` checks out
        assert_eq!(mismatches, 1, "{:?}", errors);
        assert_eq!(errors.len(), 1, "{:?}", errors);
    }

    /// The other half of the same rule: a signature *is* a commitment, so a name
    /// in one that resolves to nothing is a mistake rather than a name for
    /// whatever it turns out to be. Answering `any` here would make every bound
    /// and every obligation pass and hide the typo until something unrelated
    /// failed to match.
    #[test]
    fn an_unknown_name_in_a_signature_is_reported() {
        let (errors, _) = analyze(
            r#"
function f(x: NoSuch): NoSuch
    return x
end
"#,
        );
        assert_eq!(
            errors
                .iter()
                .filter(|e| matches!(
                    e.kind,
                    DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
                ))
                .count(),
            2,
            "{errors:?}"
        );
    }

    /// A type parameter of the function itself is a name in a signature, and it
    /// resolves, so it is not this error.
    #[test]
    fn a_type_parameter_in_a_signature_is_not_unknown() {
        let (errors, _) = analyze(
            r#"
function id<T>(x: T): T
    return x
end
"#,
        );
        assert!(
            !errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
            )),
            "{errors:?}"
        );
    }

    /// The name can be nested: `array<NoSuch>` is just as much a mistake as
    /// `NoSuch`, and a type is a tree rather than a single token.
    #[test]
    fn an_unknown_name_nested_in_a_signature_is_reported() {
        let (errors, _) = analyze(
            r#"
function f(x: array<NoSuch>)
    return x
end
"#,
        );
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                DukaErrorKind::Semantic(DukaSemanticError::UnknownType(..))
            )),
            "{errors:?}"
        );
    }

    #[test]
    fn method_call_links_to_decl() {
        let src = r#"
object A
    function :foo()
        return 1
    end
end
local a: A = A.new()
a:foo()
        "#;
        let (errors, analysis) = analyze(src);
        assert!(errors.is_empty(), "{:?}", errors);
        assert_eq!(analysis.links.len(), 1);
        let link = &analysis.links[0];
        assert_eq!(analysis.objects[link.owner].name.as_ref(), "A");
        assert_eq!(analysis.objects[link.owner].methods.len(), 1);
        let decl = analysis.objects[link.owner].methods[0].span;
        assert_eq!(link.decl_span, decl);
    }

    #[test]
    fn static_factory_call() {
        let (errors, _) = analyze(
            r#"
object a
    function foo()
        return 1
    end
end
local x = a.foo()
"#,
        );
        assert!(errors.is_empty(), "{:?}", errors);
    }
}

/// What an expression's truth stands on, one item at a time.
enum Truth {
    /// A call, and whether the truth settles its answer.
    Call(Span, bool),
    /// A name compared with a literal, and whether equality is claimed.
    Const(Box<str>, ConstValue, bool),
    /// A member path compared with a literal: `a[0] == "true"`, `cfg.host ~= nil`.
    Element(Box<str>, Box<[ConstValue]>, ConstValue, bool),
}

/// The items an expression's truth stands on, and whether that truth settles
/// them one way or the other.
///
/// `holds` says whether the expression is known to hold, and it is carried
/// through the operators that distribute it: `not` flips it, a whole `and` that
/// holds hands it to both sides, and a whole `or` that fails hands the failure
/// to both sides. Where the outcome cannot be pinned on one item -- an `and`
/// that failed may have failed on either side -- nothing is read.
fn truth_items(expr: &Expr, holds: bool, out: &mut Vec<Truth>) {
    match &expr.0 {
        NodeKind::Call(..) => out.push(Truth::Call(expr.1, holds)),
        NodeKind::Unary(e, UnOp::Not) => truth_items(e, !holds, out),
        NodeKind::Binary(a, b, BinOp::And) if holds => {
            truth_items(a, true, out);
            truth_items(b, true, out);
        }
        NodeKind::Binary(a, b, BinOp::Or) if !holds => {
            truth_items(a, false, out);
            truth_items(b, false, out);
        }
        NodeKind::Binary(a, b, op @ (BinOp::Equal | BinOp::NotEqual)) => {
            let (path, value) = match (&a.0, &b.0) {
                (NodeKind::Access(p), NodeKind::Literal(v))
                | (NodeKind::Literal(v), NodeKind::Access(p)) => (p, v),
                _ => return,
            };
            let is_equal = (*op == BinOp::Equal) == holds;
            // `x == "1"` claims `x is "1"` where it holds and `x is not "1"`
            // where it fails; `x ~= "1"` is the same the other way round
            let Some((root, steps, nil_checked)) = member_steps(path) else {
                return;
            };
            if steps.is_empty() {
                out.push(Truth::Const(root, value.clone(), is_equal));
            } else {
                // `x?[0] == lit` answered with a value, so the base was there
                // to read: a base that was nil would short-circuit to nil and
                // compare against nothing here
                if nil_checked && is_equal && value != &ConstValue::Nil {
                    out.push(Truth::Const(root.clone(), ConstValue::Nil, false));
                }
                out.push(Truth::Element(root, steps, value.clone(), is_equal));
            }
        }
        _ => {}
    }
}

/// The root name a path stands on, the literal keys down to the member, and
/// whether a `?` nil-check sits in the chain; any other suffix answers nothing.
fn member_steps(path: &Path) -> Option<(Box<str>, Box<[ConstValue]>, bool)> {
    let mut steps: Vec<ConstValue> = vec![];
    let mut nil_checked = false;
    let mut cur = path;
    loop {
        match cur {
            Path::Base((name, _)) => {
                steps.reverse();
                return Some((name.clone().into_boxed_str(), steps.into(), nil_checked));
            }
            Path::Chain(base, PathSuffix::Index(idx)) => {
                steps.push(match &idx.0 {
                    NodeKind::Literal(v) => v.clone(),
                    _ => return None,
                });
                cur = base;
            }
            Path::Chain(base, PathSuffix::Dot((name, _))) => {
                steps.push(ConstValue::String(
                    name.as_bytes().to_vec().into_boxed_slice(),
                ));
                cur = base;
            }
            // `x?[0]` walks the same member a `[0]` does -- the check only
            // says what happens when the base is missing, and the comparison
            // it sits under is what reads that
            Path::Chain(base, PathSuffix::NilCheck(_)) => {
                nil_checked = true;
                cur = base;
            }
            _ => return None,
        }
    }
}

/// A promise read the other way round.
///
/// A guarantee the flow cannot invert -- "not a sub type of", "does not have
/// the member" -- has no negation here, and nothing is known where one is
/// asked for: reading a failed condition as a claim about a name is only done
/// when the claim is exactly the opposite of what the call promised.
fn negate(c: &Constraint) -> Option<Constraint> {
    match c {
        Constraint::Is(s, t) => Some(Constraint::IsNot(s.clone(), t.clone())),
        Constraint::IsNot(s, t) => Some(Constraint::Is(s.clone(), t.clone())),
        _ => None,
    }
}

/// Whether comparing a value of this type with a literal is exact.
///
/// `==` is a method when either side is table-shaped -- a metatable may say
/// a table equals anything -- so a type that could be one of those proves
/// nothing when compared. Everything else compares as itself.
fn cannot_mask_equality(t: &Type) -> bool {
    match t {
        Type::Nil | Type::Bool | Type::Int | Type::Float | Type::String => true,
        Type::Literal(_) => true,
        // an array carries no metatable -- `__eq` is looked up on tables and
        // userdata only -- so a tuple or array answers as itself
        Type::TypeTuple(..) | Type::Array(..) => true,
        // a function or coroutine holds no metatable -- `set_metatable` takes
        // a table -- so compared to a literal they answer as themselves, and a
        // recursive alias answers as whatever it wraps
        Type::Function(..) | Type::Coroutine => true,
        Type::Rec(inner) => cannot_mask_equality(inner),
        Type::Union(us) => us.iter().all(cannot_mask_equality),
        _ => false,
    }
}

/// A type with one value removed.
///
/// A union is read member by member, so removing `nil` from `bool?` leaves
/// `bool` and removing `true` from it leaves `false`. `bool` is the one scalar
/// whose every value can be named: `int` minus `1` is still `int`, but `bool`
/// minus `true` is exactly `false`.
fn without(declared: &Type, removed: &Type) -> Type {
    let mut kept = vec![];
    without_into(declared, removed, &mut kept);
    Type::from_union_kept(kept)
}

fn without_into(declared: &Type, removed: &Type, kept: &mut Vec<Type>) {
    if declared == removed {
        return;
    }
    match declared {
        Type::Union(us) => {
            for member in us {
                without_into(member, removed, kept);
            }
        }
        Type::Bool => {
            let only = match removed {
                Type::Literal(ConstValue::Bool(b)) => Type::Literal(ConstValue::Bool(!b)),
                _ => Type::Bool,
            };
            if !kept.contains(&only) {
                kept.push(only);
            }
        }
        Type::Literal(ConstValue::Bool(_)) if matches!(removed, Type::Bool) => {}
        _ => {
            if !kept.contains(declared) {
                kept.push(declared.clone());
            }
        }
    }
}

/// Whether a type has a member called this.
///
/// The class table is read here rather than `Type::field`, which only knows a
/// record: a class member may be written on any class above it, and the chain
/// is a walk through that table rather than a single comparison.
fn has_member(t: &Type, name: &str, objects: &[ObjectType]) -> bool {
    match t {
        Type::Object { id, .. } => {
            let mut cur = Some(*id);
            while let Some(i) = cur {
                let Some(o) = objects.get(i) else {
                    return false;
                };
                if o.members.iter().any(|m| &*m.name == name)
                    || o.methods.iter().any(|m| &*m.name == name)
                {
                    return true;
                }
                cur = o.base;
            }
            false
        }
        Type::TypeTable(..) => t.field(name).is_some(),
        // a table holds whatever was put in it, and a variable is not decided
        Type::Table(..) | Type::Param(_) | Type::Any => true,
        Type::Union(us) => us.iter().any(|m| has_member(m, name, objects)),
        _ => false,
    }
}

/// Whether a type mentions this type variable.
fn mentions_param_named(t: &Type, name: &str) -> bool {
    match t {
        Type::Param(n) => &**n == name,
        Type::Array(Some(i)) => mentions_param_named(i, name),
        Type::Table(k, v, mt) => {
            k.as_deref().is_some_and(|t| mentions_param_named(t, name))
                || v.as_deref().is_some_and(|t| mentions_param_named(t, name))
                || mt.as_deref().is_some_and(|t| mentions_param_named(t, name))
        }
        Type::Object { args, .. } => args.iter().any(|a| mentions_param_named(a, name)),
        Type::Union(us) => us.iter().any(|u| mentions_param_named(u, name)),
        Type::TypeTuple(items, _) => items.iter().any(|i| mentions_param_named(i, name)),
        Type::TypeTable(fs, _) => fs.iter().any(|(_, v)| mentions_param_named(v, name)),
        Type::Rec(i) => mentions_param_named(i, name),
        _ => false,
    }
}

/// The same constraint about a different subject.
fn subject_of(c: &Constraint, s: Subject) -> Constraint {
    match c {
        Constraint::Is(_, t) => Constraint::Is(s, t.clone()),
        Constraint::IsNot(_, t) => Constraint::IsNot(s, t.clone()),
        Constraint::Sub(_, t) => Constraint::Sub(s, t.clone()),
        Constraint::Has(_, n) => Constraint::Has(s, n.clone()),
        Constraint::Instance(_, t) => Constraint::Instance(s, t.clone()),
        Constraint::Never(_) => Constraint::Never(s),
        Constraint::And(cs) => {
            Constraint::And(cs.iter().map(|c| subject_of(c, s.clone())).collect())
        }
        Constraint::Or(cs) => Constraint::Or(cs.iter().map(|c| subject_of(c, s.clone())).collect()),
        Constraint::Not(c) => Constraint::Not(Box::new(subject_of(c, s))),
    }
}

/// Answers the type variables a guarantee is written about.
fn resolve_against(c: &Constraint, solution: &solver::Solution) -> Constraint {
    let s = c.subject().cloned().unwrap_or(Subject::Arg(0));
    let r = |t: &Type| solution.substitute(t);
    match c {
        Constraint::Is(_, t) => Constraint::Is(s, r(t)),
        Constraint::IsNot(_, t) => Constraint::IsNot(s, r(t)),
        Constraint::Sub(_, t) => Constraint::Sub(s, r(t)),
        Constraint::Instance(_, t) => Constraint::Instance(s, r(t)),
        other => other.clone(),
    }
}

fn element_type_at(t: &Type, index: &ConstValue) -> Option<Type> {
    match (t, index) {
        (Type::TypeTuple(items, _), ConstValue::Int(i)) => {
            let i = usize::try_from(*i).ok()?;
            items.get(i).map(|t| t.clone())
        }
        (Type::Array(Some(inner)), _) => Some((**inner).clone()),
        (Type::TypeTable(fields, _), k) => fields
            .iter()
            .find(|(field, _)| field == k)
            .map(|(_, v)| (**v).clone()),
        (Type::Rec(inner), _) => element_type_at(inner, index),
        _ => None,
    }
}

fn element_can_equal(elem: &Type, value: &ConstValue) -> bool {
    match elem {
        Type::Literal(v) => v == value,
        Type::Union(us) => us.iter().any(|m| element_can_equal(m, value)),
        Type::Int | Type::Float => matches!(value, ConstValue::Int(_) | ConstValue::Float(_)),
        Type::String => matches!(value, ConstValue::String(_)),
        Type::Bool => matches!(value, ConstValue::Bool(_)),
        Type::Nil => matches!(value, ConstValue::Nil),
        _ => true,
    }
}

/// Narrows a position by a claim about what it equals, keeping any member
/// that could carry a metatable and answer for anything.
fn narrow_pos(pos: &Type, value: &ConstValue, is_equal: bool) -> Type {
    let keeps = |m: &Type| {
        if !cannot_mask_equality(m) {
            return true;
        }
        if is_equal {
            element_can_equal(m, value)
        } else {
            !matches!(m, Type::Literal(v) if *v == *value)
                && !(value == &ConstValue::Nil && matches!(m, Type::Nil))
        }
    };
    match pos {
        Type::Union(us) => Type::from_union_kept(us.iter().filter(|m| keeps(m)).cloned().collect()),
        _ if keeps(pos) => pos.clone(),
        _ => Type::Never,
    }
}

/// Puts a narrowed member back into its holder, or nothing where one type is
/// shared across every index or key -- one check says nothing about the rest.
fn write_pos(t: &Type, key: &ConstValue, new: &Type) -> Option<Type> {
    match (t, key) {
        (Type::TypeTable(fields, mt), k) => {
            let mut out = fields.clone();
            let slot = out.iter_mut().find(|(field, _)| field == k)?;
            *slot.1 = new.clone();
            Some(Type::TypeTable(out, mt.clone()))
        }
        (Type::TypeTuple(items, rest), ConstValue::Int(i)) => {
            let i = usize::try_from(*i).ok()?;
            let mut out = items.clone();
            *out.get_mut(i)? = new.clone();
            Some(Type::TypeTuple(out, *rest))
        }
        (Type::Rec(inner), _) => Some(Type::Rec(Box::new(write_pos(inner, key, new)?))),
        _ => None,
    }
}

/// Narrows the member a path reaches and hands back the root rebuilt with it;
/// a union keeps every member that still works and answers nothing where the
/// walk cannot be taken.
fn narrow_by_path(
    base: &Type,
    steps: &[ConstValue],
    value: &ConstValue,
    is_equal: bool,
) -> Option<Type> {
    if let Type::Union(us) = base {
        let kept: Vec<Type> = us
            .iter()
            .filter_map(|m| match narrow_by_path(m, steps, value, is_equal) {
                Some(Type::Never) => None,
                Some(narrowed) => Some(narrowed),
                None => Some(m.clone()),
            })
            .collect();
        return Some(Type::from_union_kept(kept));
    }
    let (key, rest) = steps.split_first()?;
    let pos = element_type_at(base, key)?;
    if rest.is_empty() {
        let narrowed = narrow_pos(&pos, value, is_equal);
        if narrowed == Type::Never {
            return Some(Type::Never);
        }
        return match write_pos(base, key, &narrowed) {
            Some(written) => Some(written),
            None => Some(base.clone()),
        };
    }
    let inner = narrow_by_path(&pos, rest, value, is_equal)?;
    if inner == Type::Never {
        return Some(Type::Never);
    }
    if inner == pos {
        return Some(base.clone());
    }
    write_pos(base, key, &inner)
}
