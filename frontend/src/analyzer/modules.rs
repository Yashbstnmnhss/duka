use duka_shared::attributes::Attributes;
use duka_shared::constants::ctype;
use duka_shared::types::SourceName;
use duka_shared::{
    config::{DukaAnalyzerConfig, DukaLexerConfig, DukaParserConfig},
    dtype::Type,
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{DukaAnalyzer, DukaLexer, SourceInfo},
    utils::{SymbolTableViewer, SymbolType},
    value::ConstValue,
};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path as SourcePath;
use std::sync::Arc;

use crate::{
    analyzer::prelude::inject_type_prelude,
    analyzer::typechecker::ModuleValue,
    analyzer::{AnalyzerData, ScopeAnalysis, ScopeAnalyzer, TypeChecker, TypeEval, Visit, Visitor},
    lexer::LexerWithMacro,
    parser::{
        Parser,
        ast::{
            Block, DukaChunk, Expr, Field, FuncBody, If, IfClause, Linq, LinqClause, Match, Node,
            NodeKind, ObjectDef, ObjectProperty, Param, Path, PathSuffix, Pattern, PatternTerm,
            Stmt, TypeDesc,
        },
    },
};

#[derive(Debug, Clone, PartialEq)]
pub struct DukaSource {
    pub name: Box<str>,
    pub path: Option<Arc<SourcePath>>,
    pub source: Arc<[u8]>,
}
pub trait DukaSourceProvider {
    fn load(&self, name: &str, caller_path: Option<&SourcePath>) -> Option<DukaSource>;
}

#[derive(Debug, Clone)]
pub enum ExportedTypeKind {
    Object(usize),
    Alias(usize),
    TypeFn(usize),
    InlineFn(usize),
}

#[derive(Debug, Clone)]
pub struct ModuleType {
    pub key: Box<str>,
    pub source: Arc<SourceInfo>,
    pub analysis: Arc<ScopeAnalysis>,
    pub exported: HashMap<Box<str>, ExportedTypeKind>,
    pub value: ModuleValue,
}
impl ModuleType {
    #[inline]
    pub fn get(&self, name: &str) -> Option<ExportedTypeKind> {
        self.exported.get(name).cloned()
    }

    /// Type `require` hands back to the importer
    #[inline]
    pub fn value_type(&self) -> Type {
        self.value.ty.clone()
    }

    /// Type of a single `export`ed value
    #[inline]
    pub fn value_member(&self, name: &str) -> Option<&Type> {
        self.value.members.get(name)
    }

    /// What an attribute written on an `export`ed declaration says about it
    #[inline]
    pub fn value_attributes(&self, name: &str) -> Option<Attributes> {
        self.value.attributes.get(name).cloned()
    }

    #[inline]
    pub fn value_is_declared(&self, name: &str) -> bool {
        self.value
            .attributes
            .get(name)
            .is_some_and(|a| a.has_declare())
    }

    /// Every name this module answers for in a type position: what it exports as
    /// a type, and what it only declared, since a declaration is a shape too.
    ///
    /// A name is offered here only if there is something to answer with. An
    /// ordinary export is not: what a type position means by it is the type the
    /// module exports, and it exports none.
    pub fn type_position_names(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.exported.keys().map(|k| &**k).collect();
        out.extend(
            self.value
                .attributes
                .iter()
                .filter(|(_, a)| a.has_declare())
                .map(|(name, _)| &**name),
        );
        out.sort_unstable();
        out.dedup();
        out
    }
}

pub type ModuleMap = HashMap<Box<str>, ModuleType>;

pub struct ModuleBuild {
    pub modules: ModuleMap,
    pub data: AnalyzerData,
    pub errors: Vec<DukaSpannedError>,
}

pub fn build_module_types(
    entry: &DukaChunk,
    entry_data: AnalyzerData,
    config: DukaAnalyzerConfig,
    lexer_cfg: DukaLexerConfig,
    parser_cfg: DukaParserConfig,
    provider: &dyn DukaSourceProvider,
) -> ModuleBuild {
    let mut cache = ModuleBuildCache::default();
    build_module_types_cached(
        entry, entry_data, config, lexer_cfg, parser_cfg, provider, &mut cache,
    )
}

/// FNV-1a fingerprint of source bytes
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Module build cache: when dependency source fingerprints are unchanged, the
/// previous analysis result is reused
#[derive(Default)]
pub struct ModuleBuildCache {
    fingerprint: Vec<(Box<str>, u64)>,
    modules: ModuleMap,
    refs: HashMap<Box<str>, Vec<(String, Span)>>,
}

fn walk_fingerprint(
    ref_names: &[(String, Span)],
    caller_path: Option<&SourcePath>,
    provider: &dyn DukaSourceProvider,
    cache_refs: &HashMap<Box<str>, Vec<(String, Span)>>,
    cache_modules: &ModuleMap,
    out: &mut Vec<(Box<str>, u64)>,
    visited: &mut HashSet<Box<str>>,
) -> bool {
    for (name, _) in ref_names {
        let Some(DukaSource {
            name: key,
            path,
            source: bytes,
        }) = provider.load(name, caller_path)
        else {
            return false;
        };
        if !visited.insert(key.clone()) {
            continue;
        }
        if !cache_modules.contains_key(&key) || !cache_refs.contains_key(&key) {
            return false;
        }
        out.push((key.clone(), fnv1a(&bytes)));
        let child_refs = &cache_refs[&key];
        if !walk_fingerprint(
            child_refs,
            path.as_deref(),
            provider,
            cache_refs,
            cache_modules,
            out,
            visited,
        ) {
            return false;
        }
    }
    true
}

pub fn build_module_types_cached(
    entry: &DukaChunk,
    entry_data: AnalyzerData,
    config: DukaAnalyzerConfig,
    lexer_cfg: DukaLexerConfig,
    parser_cfg: DukaParserConfig,
    provider: &dyn DukaSourceProvider,
    cache: &mut ModuleBuildCache,
) -> ModuleBuild {
    let entry_key: Box<str> = entry.source_info.name.to_string().into_boxed_str();
    let entry_refs = collect_refs(entry);

    if !cache.modules.is_empty() && !cache.fingerprint.is_empty() {
        let mut fp = vec![];
        let mut visited = HashSet::new();
        if walk_fingerprint(
            &entry_refs,
            entry.source_info.name.path(),
            provider,
            &cache.refs,
            &cache.modules,
            &mut fp,
            &mut visited,
        ) {
            fp.sort();
            let mut expect = cache.fingerprint.clone();
            expect.sort();
            if fp == expect {
                let mut modules = cache.modules.clone();
                let exported = collect_exports(entry, &entry_data.1);
                modules.insert(
                    entry_key.clone(),
                    ModuleType {
                        key: entry_key.clone(),
                        source: Arc::new(entry.source_info.clone()),
                        analysis: Arc::new(ScopeAnalysis::default()),
                        exported,
                        value: ModuleValue::default(),
                    },
                );
                cache.refs.insert(entry_key, entry_refs);
                return ModuleBuild {
                    modules,
                    data: entry_data,
                    errors: vec![],
                };
            }
        }
    }

    let mut modules = ModuleMap::new();
    let mut loading = HashSet::new();
    let mut errors = vec![];
    let mut refs: HashMap<Box<str>, Vec<(String, Span)>> = HashMap::new();
    refs.insert(entry_key.clone(), entry_refs);
    let exported = collect_exports(entry, &entry_data.1);
    modules.insert(
        entry_key.clone(),
        ModuleType {
            key: entry_key.clone(),
            source: Arc::new(entry.source_info.clone()),
            analysis: Arc::new(ScopeAnalysis::default()),
            exported,
            value: ModuleValue::default(),
        },
    );
    for (name, span) in collect_refs(entry) {
        collect_module(
            &name,
            entry.source_info.name.path(),
            span,
            config.clone(),
            lexer_cfg.clone(),
            parser_cfg.clone(),
            provider,
            &mut modules,
            &mut loading,
            &mut errors,
            &mut refs,
        );
    }

    let mut fp = vec![];
    let mut visited = HashSet::new();
    if !walk_fingerprint(
        &refs[&entry_key],
        entry.source_info.name.path(),
        provider,
        &refs,
        &modules,
        &mut fp,
        &mut visited,
    ) {
        fp.clear();
    }
    fp.sort();

    cache.fingerprint = fp;
    cache.refs = refs;
    cache.modules = modules.clone();

    ModuleBuild {
        modules,
        data: entry_data,
        errors,
    }
}

fn collect_module(
    name: &str,
    caller_path: Option<&std::path::Path>,
    span: Span,
    config: DukaAnalyzerConfig,
    lexer_cfg: DukaLexerConfig,
    parser_cfg: DukaParserConfig,
    provider: &dyn DukaSourceProvider,
    modules: &mut ModuleMap,
    loading: &mut HashSet<Box<str>>,
    errors: &mut Vec<DukaSpannedError>,
    refs: &mut HashMap<Box<str>, Vec<(String, Span)>>,
) {
    let Some(DukaSource {
        name: key,
        path,
        source: src,
    }) = provider.load(name, caller_path)
    else {
        return;
    };
    if modules.contains_key(&key) {
        return;
    }
    if !loading.insert(key.clone()) {
        errors.push(DukaSpannedError {
            level: Default::default(),
            kind: DukaSemanticError::CircularRequire(key.clone()).into(),
            span,
            related: [].into(),
            source_info: Arc::new(SourceInfo {
                name: SourceName::Virtual(key.into()),
                source: Arc::new([]),
            }),
        });
        return;
    }
    let source = String::from_utf8_lossy(&src).into_owned();
    let lexer = LexerWithMacro::new(
        Cursor::new(source.as_str()),
        if let Some(path) = path.clone() {
            SourceName::File(key.clone().into(), path)
        } else {
            SourceName::Virtual(key.clone().into())
        },
        lexer_cfg.clone(),
    );
    let stream = match lexer.tokenize() {
        Ok(s) => s,
        Err(e) => {
            errors.push(e);
            loading.remove(&key);
            return;
        }
    };
    // a dependency is parsed here too, so it has to be told the bang keywords
    // the same way the entry chunk is
    let chunk = match crate::expander::BangExpanderRegistry::new()
        .names()
        .register_with(Parser::from_tokens(stream, parser_cfg.clone()))
        .parse_into_chunk()
    {
        Ok(c) => c,
        Err(e) => {
            errors.push(e);
            loading.remove(&key);
            return;
        }
    };
    let (data, errs) = ScopeAnalyzer.analyze(&chunk, config.clone());
    errors.extend(errs);
    refs.insert(key.clone(), collect_refs(&chunk));
    for (n, s) in collect_refs(&chunk) {
        collect_module(
            &n,
            path.as_deref(),
            s,
            config.clone(),
            lexer_cfg.clone(),
            parser_cfg.clone(),
            provider,
            modules,
            loading,
            errors,
            refs,
        );
    }
    let exported = collect_exports(&chunk, &data.1);
    let (_, analysis) = data;
    // a module is a program of its own: give it the same passes the entry gets,
    // with the modules it depends on already resolved
    let mut full = ScopeAnalysis {
        modules: modules.clone(),
        ..analysis
    };
    errors.extend(inject_type_prelude(&mut full));
    let (typed, eval_errs) =
        TypeEval.analyze_with_provider(&chunk, (config.clone(), full), Some(provider));
    errors.extend(eval_errs);
    let (data, type_errs, value) =
        TypeChecker.analyze_module(&chunk, typed, Some(provider), Some(&key));
    errors.extend(type_errs);
    let m = ModuleType {
        key: key.clone(),
        source: Arc::new(chunk.source_info.clone()),
        analysis: Arc::new(data.1),
        exported,
        value,
    };
    loading.remove(&key);
    modules.insert(key, m);
}

fn collect_refs(chunk: &DukaChunk) -> Vec<(String, Span)> {
    let mut out = vec![];
    let mut walker = RefWalker { out: &mut out };
    chunk.visit(&mut walker);
    out
}

struct RefWalker<'a> {
    out: &'a mut Vec<(String, Span)>,
}

impl Visitor for RefWalker<'_> {
    fn visit_node(&mut self, stmt: &Stmt) {
        walk_stmt(stmt, self.out);
    }
}

fn walk_stmt(stmt: &Stmt, out: &mut Vec<(String, Span)>) {
    match &stmt.0 {
        NodeKind::TypeAlias(_, ty) => walk_type_value(ty, out),
        NodeKind::TypeFunction(_, body) => walk_func_body(body, out),
        NodeKind::Object(obj) => walk_object(obj, out),
        NodeKind::Define(attrs, exprs, _, _) => {
            for attr in attrs.iter() {
                if let Some(ty) = &attr.0.2 {
                    walk_type_value(ty, out);
                }
            }
            for e in exprs.iter() {
                walk_expr(e, out);
            }
        }
        NodeKind::FunctionDecl(_, _, body, _) => walk_func_body(body, out),
        NodeKind::Export(inner) => walk_stmt(inner, out),
        NodeKind::Assign(_, exprs) => {
            for e in exprs.iter() {
                walk_expr(e, out);
            }
        }
        NodeKind::While(cond, body, ..) => {
            walk_expr(cond, out);
            walk_block(body, out);
        }
        NodeKind::Do(body, ..) => walk_block(body, out),
        NodeKind::ForNumeric(_, start, limit, step, body, ..) => {
            walk_expr(start, out);
            walk_expr(limit, out);
            if let Some(step) = step {
                walk_expr(step, out);
            }
            walk_block(body, out);
        }
        NodeKind::ForGeneric(_, exprs, body, ..) => {
            for e in exprs.iter() {
                walk_expr(e, out);
            }
            walk_block(body, out);
        }
        NodeKind::Match(m) => walk_match(m, out),
        NodeKind::Return(exprs, ..) => {
            for e in exprs.iter() {
                walk_expr(e, out);
            }
        }
        NodeKind::If(if_) => walk_if(if_, out),
        _ => {}
    }
}

fn walk_block(block: &Block, out: &mut Vec<(String, Span)>) {
    for s in block.0.iter() {
        walk_stmt(s, out);
    }
}

fn walk_func_body(body: &FuncBody, out: &mut Vec<(String, Span)>) {
    for p in body.0.iter() {
        if let Param::Typed(_, ty) = p {
            walk_type_value(ty, out);
        }
    }
    for tp in body.1.iter() {
        if let Some(ty) = &tp.1 {
            walk_type_value(ty, out);
        }
    }
    if let Some(ty) = &body.2 {
        for t in ty.tys.iter() {
            walk_type_value(t, out);
        }
    }
    walk_block(&body.4, out);
}

fn walk_object(obj: &ObjectDef, out: &mut Vec<(String, Span)>) {
    for tp in obj.type_params.iter() {
        if let Some(ty) = &tp.1 {
            walk_type_value(ty, out);
        }
    }
    for prop in obj.properties.iter() {
        match prop {
            ObjectProperty::NameValue(_, _, ty) => {
                if let Some(ty) = ty {
                    walk_type_value(ty, out);
                }
            }
            ObjectProperty::KeyValue(_, _, ty) => {
                if let Some(ty) = ty {
                    walk_type_value(ty, out);
                }
            }
        }
    }
    for (_, _, body) in obj.static_methods.iter() {
        walk_func_body(body, out);
    }
    for (_, _, body) in obj.methods.iter() {
        walk_func_body(body, out);
    }
}

fn walk_match(m: &Match, out: &mut Vec<(String, Span)>) {
    walk_expr(&m.0, out);
    for clause in m.1.iter() {
        walk_pattern(&clause.0, out);
        walk_block(&clause.1, out);
    }
    if let Some(else_block) = &m.2 {
        walk_block(else_block, out);
    }
}

fn walk_term(p: &PatternTerm, out: &mut Vec<(String, Span)>) {
    match p {
        PatternTerm::Bind(_, ty) => {
            if let Some(ty) = ty {
                walk_type_value(ty, out);
            }
        }
        PatternTerm::Call(_, e, sub) => {
            walk_expr(e, out);
            if let Some(sub) = sub {
                walk_term(sub, out);
            }
        }
        PatternTerm::Compare(_, e) => walk_expr(e, out),
        _ => {}
    }
}

fn walk_pattern(p: &Pattern, out: &mut Vec<(String, Span)>) {
    walk_term(&p.0, out);
    if let Some(guard) = &p.1 {
        walk_expr(guard, out);
    }
}

fn walk_linq(l: &Linq, out: &mut Vec<(String, Span)>) {
    for clause in l.0.iter() {
        match clause {
            LinqClause::Where(e) => walk_expr(e, out),
            LinqClause::From(_, e) => walk_expr(e, out),
        }
    }
    walk_expr(&l.1, out);
}

fn walk_if(if_: &If, out: &mut Vec<(String, Span)>) {
    walk_if_clause(&if_.0, out);
    for clause in if_.1.iter() {
        walk_if_clause(clause, out);
    }
    if let Some(else_block) = &if_.2 {
        walk_block(else_block, out);
    }
}

fn walk_if_clause(clause: &IfClause, out: &mut Vec<(String, Span)>) {
    walk_block(&clause.0, out);
    walk_expr(&clause.1, out);
}

fn walk_expr(expr: &Expr, out: &mut Vec<(String, Span)>) {
    match &expr.0 {
        NodeKind::BangDo(n) => {
            walk_expr(&n.context, out);
            walk_block(&n.body, out);
        }
        NodeKind::Call(f, args) => {
            // Both spellings of "load this module", and they are both expressions
            // now -- `RequireType("m")` in a type position and `require("m")` at a
            // value position are the same shape with two names. They used to be
            // two shapes, found by two walks, and the type one was easy to drop:
            // when it was, the module was never loaded, and every annotation that
            // named it quietly read as `any` instead of failing.
            if let NodeKind::Access(p) = &f.0
                && let Path::Base((name, _)) = p.as_ref()
                && (name.as_str() == "require" || name.as_str() == ctype::REQUIRE)
                && let [
                    Node(
                        NodeKind::TypeLit(TypeDesc::Pure(Type::Literal(ConstValue::String(bytes)))),
                        _,
                    ),
                ] = &args[..]
            {
                out.push((String::from_utf8_lossy(bytes).into_owned(), expr.1));
            }
            if let NodeKind::Access(p) = &f.0
                && let Path::Base((name, _)) = p.as_ref()
                && name.as_str() == "require"
                && let Some(Node(NodeKind::Literal(ConstValue::String(bytes)), _)) = args.first()
            {
                out.push((String::from_utf8_lossy(bytes).into_owned(), expr.1));
            }
            walk_expr(f, out);
            for a in args.iter() {
                walk_expr(a, out);
            }
        }
        NodeKind::Access(path) => walk_path(path, out),
        NodeKind::Do(block, ..) => walk_block(block, out),
        NodeKind::Table(fields) => {
            for f in fields.iter() {
                match f {
                    Field::Value(e) => walk_expr(e, out),
                    Field::KeyValue(k, v) => {
                        walk_expr(k, out);
                        walk_expr(v, out);
                    }
                    Field::NameValue(_, v) => walk_expr(v, out),
                }
            }
        }
        NodeKind::Array(items) => {
            for e in items.iter() {
                walk_expr(e, out);
            }
        }
        NodeKind::Function(body) => walk_func_body(body, out),
        NodeKind::Unary(e, _) => walk_expr(e, out),
        NodeKind::Binary(a, b, _) => {
            walk_expr(a, out);
            walk_expr(b, out);
        }
        NodeKind::If(if_) => walk_if(if_, out),
        NodeKind::Match(m) => walk_match(m, out),
        NodeKind::Linq(l) => walk_linq(l, out),
        NodeKind::TypeLit(tv) => walk_type_value(tv, out),
        NodeKind::Empty
        | NodeKind::VarArg
        | NodeKind::Literal(_)
        | NodeKind::SysCall(_)
        | NodeKind::BangCollected(_) => {}
        // Statement-only forms reach `walk_stmt`, which is what the visitor calls
        // for a node; `walk_expr` only descends into what a statement holds.
        NodeKind::Expr(_)
        | NodeKind::Label(_)
        | NodeKind::Goto(_)
        | NodeKind::Break(..)
        | NodeKind::Continue(_)
        | NodeKind::Return(..)
        | NodeKind::Object(_)
        | NodeKind::Export(_)
        | NodeKind::ForNumeric(..)
        | NodeKind::ForGeneric(..)
        | NodeKind::While(..)
        | NodeKind::Assign(..)
        | NodeKind::Define(..)
        | NodeKind::Destructing(..)
        | NodeKind::FunctionDecl(..)
        | NodeKind::TypeAlias(..)
        | NodeKind::TypeFunction(..)
        | NodeKind::InlineTypeFunction(..) => {}
    }
}

fn walk_path(path: &Path, out: &mut Vec<(String, Span)>) {
    match path {
        Path::Expr(e) => walk_expr(e, out),
        Path::Chain(p, PathSuffix::Index(e)) => {
            walk_path(p, out);
            walk_expr(e, out);
        }
        Path::Chain(p, _) => walk_path(p, out),
        Path::Base(_) => {}
    }
}

fn walk_type_value(tv: &TypeDesc, out: &mut Vec<(String, Span)>) {
    match tv {
        // An application is an expression now, so `require("m")` written as one is
        // reached by walking the expression -- which is where a `require` written
        // with operators was already reached. Same walk, one shape.
        TypeDesc::TypeOf(expr, _) => walk_expr(expr, out),
        TypeDesc::Splice(..) => {}
        // a `require` can be written inside a type-level expression too
        TypeDesc::Expr(expr) => walk_expr(expr, out),
        TypeDesc::Generic(_, args, _) => {
            for a in args.iter() {
                walk_type_value(a, out);
            }
        }
        TypeDesc::Array(e) => {
            if let Some(e) = e {
                walk_type_value(e, out);
            }
        }
        TypeDesc::Table(k, v) => {
            if let Some(k) = k {
                walk_type_value(k, out);
            }
            if let Some(v) = v {
                walk_type_value(v, out);
            }
        }
        TypeDesc::Union(ts) => {
            for t in ts.iter() {
                walk_type_value(t, out);
            }
        }
        TypeDesc::TypeTuple(ts, _) => {
            for t in ts.iter() {
                walk_type_value(t, out);
            }
        }
        TypeDesc::TypeTable(ts) => {
            for f in ts.iter() {
                walk_type_value(&f.ty, out);
            }
        }
        TypeDesc::Function(ft) => {
            if let Some(ft) = ft {
                for p in ft.params.iter() {
                    walk_type_value(p, out);
                }
                for r in ft.returns.iter() {
                    walk_type_value(r, out);
                }
            }
        }
        TypeDesc::Pure(_)
        | TypeDesc::FnLit(..)
        | TypeDesc::NonNil(_)
        | TypeDesc::Nilable(_)
        | TypeDesc::Named(..)
        | TypeDesc::Rec(_) => {}
    }
}

fn collect_exports(
    chunk: &DukaChunk,
    analysis: &ScopeAnalysis,
) -> HashMap<Box<str>, ExportedTypeKind> {
    let viewer = SymbolTableViewer::new(&analysis.symbols);
    let mut exported = HashMap::new();
    collect_exports_block(&chunk.block, &viewer, &mut exported);
    exported
}

fn collect_exports_block(
    block: &Block,
    viewer: &SymbolTableViewer<'_>,
    exported: &mut HashMap<Box<str>, ExportedTypeKind>,
) {
    for s in block.0.iter() {
        collect_exports_stmt(s, viewer, exported);
    }
}

fn collect_exports_stmt(
    stmt: &Stmt,
    viewer: &SymbolTableViewer<'_>,
    exported: &mut HashMap<Box<str>, ExportedTypeKind>,
) {
    let NodeKind::Export(inner) = &stmt.0 else {
        return;
    };
    match &inner.0 {
        NodeKind::TypeAlias((name, _), _) => {
            if let Some(sym) = viewer.lookup_anywhere(&name.0.0)
                && let SymbolType::TypeAlias(id) = sym.symbol_type
            {
                exported.insert(
                    name.0.0.clone().into_boxed_str(),
                    ExportedTypeKind::Alias(id),
                );
            }
        }
        NodeKind::TypeFunction((name, _), _) => {
            if let Some(sym) = viewer.lookup_anywhere(&name.0.0)
                && let SymbolType::TypeFunction(id) = sym.symbol_type
            {
                exported.insert(
                    name.0.0.clone().into_boxed_str(),
                    ExportedTypeKind::TypeFn(id),
                );
            }
        }
        NodeKind::InlineTypeFunction((name, _), _, _) => {
            if let Some(sym) = viewer.lookup_anywhere(&name.0.0)
                && let SymbolType::InlineTypeFunction(id) = sym.symbol_type
            {
                exported.insert(
                    name.0.0.clone().into_boxed_str(),
                    ExportedTypeKind::InlineFn(id),
                );
            }
        }
        NodeKind::Object(obj) => {
            if let Some(sym) = viewer.lookup_anywhere(&obj.name.0)
                && let SymbolType::ObjectClass(id) = sym.symbol_type
            {
                exported.insert(
                    obj.name.0.clone().into_boxed_str(),
                    ExportedTypeKind::Object(id),
                );
            }
        }
        _ => {}
    }
}

/// 璁㎡bject閮藉彉Any
/// Types that cross a module boundary, with anything the importer cannot answer
/// widened to `Any` first.
///
/// A class used to be widened here, because an object names its file's own table
/// of classes by an index and that index means nothing anywhere else. It carries
/// the module it was declared in now, so an importer looks the class up there
/// and the class survives the crossing: `m.Point.new(1, 2)` is a `Point` on the
/// other side of a `require`, with its fields.
///
/// What is left to widen is a class whose module is gone, which is what a type
/// read out of a module that could not be loaded looks like. The walk into
/// containers stays, because a class can sit inside one.
///
/// ```
/// use duka_frontend::analyzer::modules::sanitize_foreign;
/// use duka_shared::dtype::Type;
///
/// let obj = Type::Object {
///     id: 0,
///     name: "A".into(),
///     base: None,
///     args: [].into(),
///     module: Some("m.duka".into()),
///     mt: None,
/// };
///
/// // a class that says where it lives crosses as itself
/// assert_eq!(sanitize_foreign(obj.clone()), obj);
/// assert_eq!(
///     sanitize_foreign(Type::Array(Some(obj.clone().into()))),
///     Type::Array(Some(obj.clone().into()))
/// );
///
/// // and one that does not has nothing left to be asked about
/// let lost = Type::Object {
///     id: 0,
///     name: "A".into(),
///     base: None,
///     args: [].into(),
///     module: None,
///     mt: None,
/// };
/// assert_eq!(sanitize_foreign(lost), Type::Any);
///
/// // scalars and open containers are left alone
/// assert_eq!(sanitize_foreign(Type::Int), Type::Int);
/// assert_eq!(sanitize_foreign(Type::Array(None)), Type::Array(None));
/// ```
pub fn sanitize_foreign(t: Type) -> Type {
    match t {
        // a class with no module was never stamped with one, so nothing on this
        // side can say which table its id indexes -- and answering `any` is the
        // reading that never rejects a call the other side would accept
        Type::Object { module: None, .. } => Type::Any,
        Type::Array(Some(inner)) => Type::Array(Some(Box::new(sanitize_foreign(*inner)))),
        Type::Array(None) => Type::Array(None),
        Type::Table(k, v, mt) => Type::Table(
            k.map(|k| Box::new(sanitize_foreign(*k))),
            v.map(|v| Box::new(sanitize_foreign(*v))),
            mt.map(|m| Box::new(sanitize_foreign(*m))),
        ),
        Type::Union(ts) => Type::Union(ts.into_vec().into_iter().map(sanitize_foreign).collect()),
        Type::TypeTuple(ts, rest) => {
            Type::TypeTuple(ts.into_iter().map(sanitize_foreign).collect(), rest)
        }
        Type::TypeTable(ts) => Type::TypeTable(
            ts.into_iter()
                .map(|(k, v)| (k, Box::new(sanitize_foreign(*v))))
                .collect(),
        ),
        Type::Function(Some(ft)) => Type::Function(Some(duka_shared::dtype::FunctionType {
            params: ft
                .params
                .into_vec()
                .into_iter()
                .map(sanitize_foreign)
                .collect(),
            returns: ft
                .returns
                .into_vec()
                .into_iter()
                .map(sanitize_foreign)
                .collect(),
            ..ft
        })),
        other => other,
    }
}

pub fn resolve_module_type<'a>(
    modules: &'a ModuleMap,
    name: &str,
    caller_path: Option<&SourcePath>,
    provider: &dyn DukaSourceProvider,
) -> Option<&'a ModuleType> {
    if let Some(m) = modules.get(name) {
        return Some(m);
    }
    let DukaSource { name: key, .. } = provider.load(name, caller_path)?;
    modules.get(&key)
}

#[cfg(test)]
mod tests {
    use super::*;
    // brings `Parser::parse` (a `DukaParser` method) into scope
    use duka_shared::types::DukaParser;
    use std::collections::HashMap;

    use crate::analyzer::{TypeChecker, TypeEval};
    use duka_shared::config::{DukaAnalyzerConfig, DukaLexerConfig, DukaParserConfig};

    struct TestProvider {
        modules: HashMap<String, String>,
    }

    impl DukaSourceProvider for TestProvider {
        fn load(&self, name: &str, _caller_path: Option<&SourcePath>) -> Option<DukaSource> {
            let src = self.modules.get(name)?;

            Some(DukaSource {
                name: format!("m:{name}").into_boxed_str(),
                path: None,
                source: src.as_bytes().to_vec().into(),
            })
        }
    }

    /// The module map the entry reaches, so a test can ask what a module says
    /// rather than only what it makes the entry say
    fn build_modules(entry: &str, provider: &TestProvider) -> ModuleMap {
        let lexer = LexerWithMacro::new(
            Cursor::new(entry),
            SourceName::Virtual("main.duka".into()),
            Default::default(),
        );
        let chunk = Parser::parse(lexer.tokenize().unwrap(), Default::default()).unwrap();
        let (data, _errs) = ScopeAnalyzer.analyze(&chunk, DukaAnalyzerConfig::default());
        build_module_types(
            &chunk,
            data,
            DukaAnalyzerConfig::default(),
            DukaLexerConfig::default(),
            DukaParserConfig::default(),
            provider,
        )
        .modules
    }

    fn analyze_entry(entry: &str, provider: &TestProvider) -> Vec<String> {
        let lexer = LexerWithMacro::new(
            Cursor::new(entry),
            SourceName::Virtual("main.duka".into()),
            Default::default(),
        );
        let stream = lexer.tokenize().unwrap();
        let chunk = Parser::parse(stream, Default::default()).unwrap();
        let (data, errs1) = ScopeAnalyzer.analyze(&chunk, DukaAnalyzerConfig::default());
        let build = build_module_types(
            &chunk,
            data,
            DukaAnalyzerConfig::default(),
            DukaLexerConfig::default(),
            DukaParserConfig::default(),
            provider,
        );
        let mut data = build.data;
        data.1.modules = build.modules;
        let mut errors: Vec<_> = errs1.chain(build.errors).collect();
        let (data, errs) = TypeEval.analyze_with_provider(&chunk, data, Some(provider));
        errors.extend(errs);
        let (_data, errs) = TypeChecker.analyze_with_modules(&chunk, data, Some(provider));
        errors.extend(errs);
        errors.iter().map(|e| e.to_string()).collect()
    }

    fn provider(pairs: &[(&str, &str)]) -> TestProvider {
        TestProvider {
            modules: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn require_value_is_module_tail() {
        let p = provider(&[("m", "local M = {} M.hello = \"hi\" return M")]);
        let errs = analyze_entry("local m = require(\"m\") local s: string = m.hello", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn type_prelude_parses() {
        let errs = crate::analyzer::prelude::inject_type_prelude(&mut ScopeAnalysis::default());
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn pipe_call_in_required_module_is_checked() {
        let p = provider(&[(
            "m",
            "local function f(n: int) return n end local piped = (\"x\" |> f) return piped",
        )]);
        let errs = analyze_entry("local m = require(\"m\") return m", &p);
        assert!(!errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn valid_pipe_call_in_required_module_passes() {
        let p = provider(&[(
            "m",
            "local function f(n: int) return n end local piped = (10 |> f) return piped",
        )]);
        let errs = analyze_entry("local m = require(\"m\") local s: int = m", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    /// The entry's own analysis, so a test can ask what it recorded rather than
    /// only what it made the entry say
    fn analyze_entry_types(entry: &str, provider: &TestProvider) -> ScopeAnalysis {
        let lexer = LexerWithMacro::new(
            Cursor::new(entry),
            SourceName::Virtual("main.duka".into()),
            Default::default(),
        );
        let chunk = Parser::parse(lexer.tokenize().unwrap(), Default::default()).unwrap();
        let (data, _errs) = ScopeAnalyzer.analyze(&chunk, DukaAnalyzerConfig::default());
        let build = build_module_types(
            &chunk,
            data,
            DukaAnalyzerConfig::default(),
            DukaLexerConfig::default(),
            DukaParserConfig::default(),
            provider,
        );
        let mut data = build.data;
        data.1.modules = build.modules;
        let (data, _errs) = TypeEval.analyze_with_provider(&chunk, data, Some(provider));
        let (data, _errs) = TypeChecker.analyze_with_modules(&chunk, data, Some(provider));
        data.1
    }

    #[test]
    fn a_type_position_records_the_name_it_resolves() {
        let p = provider(&[
            ("m", "export type Alias = { x: int, y: string }"),
            (
                "n",
                "export type Outer = { inner: { deep: bool }, top: int }",
            ),
        ]);
        let entry = "local a: RequireType(\"m\").Alias = { x = 1, y = \"s\" }\n\
                     local b: RequireType(\"n\").Outer.inner.deep = true\n\
                     local c: int = 1\n";
        let analysis = analyze_entry_types(entry, &p);
        // lines are 1-based in a span and 0-based in `lines`
        let on = |line: u32| -> Vec<String> {
            let mut out: Vec<String> = analysis
                .type_at_span
                .iter()
                .filter(|(span, _)| span.start.line == line)
                .map(|(_, ty)| ty.to_string())
                // a record renders over several lines, which says nothing here
                .map(|ty| ty.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            out.sort();
            out
        };
        assert_eq!(
            on(1),
            vec!["{ x: int, y: string }".to_owned()],
            "the alias itself"
        );
        assert_eq!(
            on(2),
            vec![
                "bool".to_owned(),
                "{ deep: bool }".to_owned(),
                "{ inner: { deep: bool }, top: int }".to_owned(),
            ],
            "every member read off it is a reference of its own"
        );
        assert!(
            on(3).is_empty(),
            "a bare `int` is not a member, so there is no reference to record"
        );
        assert_eq!(
            analysis.type_at_span.len(),
            4,
            "{:#?}",
            analysis.type_at_span
        );
    }

    #[test]
    fn a_type_position_records_a_declaration_of_another_module_too() {
        let p = provider(&[("m", "@declare export function strlen(s: string): int")]);
        let analysis = analyze_entry_types("local g: RequireType(\"m\").strlen\n", &p);
        let recorded: Vec<String> = analysis
            .type_at_span
            .values()
            .map(|ty| ty.to_string())
            .collect();
        assert_eq!(
            recorded,
            vec!["function(string?) -> int".to_owned()],
            "{recorded:?}"
        );
    }

    #[test]
    fn a_type_position_with_no_member_records_nothing() {
        let p = provider(&[]);
        let analysis = analyze_entry_types("local n: int = 1\nlocal s: string = \"s\"\n", &p);
        assert!(
            analysis.type_at_span.is_empty(),
            "{:#?}",
            analysis.type_at_span
        );
    }

    #[test]
    fn require_value_member_type_is_checked() {
        let p = provider(&[("m", "local M = {} M.hello = \"hi\" return M")]);
        let errs = analyze_entry("local m = require(\"m\") local n: int = m.hello", &p);
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
    }

    #[test]
    fn require_value_missing_module_is_any() {
        let p = provider(&[]);
        let errs = analyze_entry(
            "local m = require(\"nope\") local s: string = m.whatever",
            &p,
        );
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn require_value_direct_member_access() {
        let p = provider(&[("m", "local M = {} M.hello = \"hi\" return M")]);
        let errs = analyze_entry("local s: string = require(\"m\").hello", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn exported_function_is_callable_with_arity() {
        let p = provider(&[("m", "export function twice(a, b) return a end")]);
        let errs = analyze_entry("local m = require(\"m\") m.twice(1)", &p);
        assert!(
            errs.iter().any(|e| e.contains("2 arguments")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn exported_function_member_type_flows() {
        let p = provider(&[("m", "export function name() return \"duka\" end")]);
        let errs = analyze_entry("local m = require(\"m\") local s: int = m.name()", &p);
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
    }

    #[test]
    fn module_without_tail_or_export_is_open_table() {
        let p = provider(&[("m", "local x = 1")]);
        let errs = analyze_entry("local m = require(\"m\") local s: string = m.anything", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn module_errors_are_reported() {
        let p = provider(&[("m", "local n: int = \"no\"")]);
        let errs = analyze_entry("local m = require(\"m\")", &p);
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
    }

    #[test]
    fn module_sees_builtin_prelude() {
        let p = provider(&[("m", "print(\"hello\") return nil")]);
        let errs = analyze_entry("local m = require(\"m\")", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn require_value_member_of_another_module() {
        let p = provider(&[
            ("a", "local M = {} M.value = require(\"b\").count return M"),
            ("b", "export function count(): int return 1 end"),
        ]);
        let errs = analyze_entry("local a = require(\"a\") local n: int = a.value()", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn require_value_member_of_another_module_rejects_mismatch() {
        let p = provider(&[
            ("a", "local M = {} M.value = require(\"b\").count return M"),
            ("b", "export function count(): int return 1 end"),
        ]);
        let errs = analyze_entry("local a = require(\"a\") local n: string = a.value()", &p);
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
    }

    #[test]
    fn require_type_alias_rejects_mismatch() {
        let p = provider(&[("m", "export type Alias = { x: int, y: string }")]);
        let errs = analyze_entry("local a: RequireType(\"m\").Alias = { x = 1 }", &p);
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
    }

    #[test]
    fn require_type_nested_member() {
        let p = provider(&[("m", "export type Alias = { x: int, y: string }")]);
        let errs = analyze_entry("local a: RequireType(\"m\").Alias.y = \"s\"", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn require_type_type_fn_call() {
        let p = provider(&[("m", "export type function Pick(a, b)\n    return a\nend")]);
        let errs = analyze_entry("local a: RequireType(\"m\").Pick(int, string) = 1", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn require_type_type_fn_rejects_mismatch() {
        let p = provider(&[("m", "export type function Pick(a, b)\n    return a\nend")]);
        let errs = analyze_entry("local a: RequireType(\"m\").Pick(int, string) = \"x\"", &p);
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
    }

    #[test]
    fn require_type_object_is_the_class_itself() {
        let p = provider(&[("m", "export object Foo\n    x = 1\nend")]);
        // what a class means in a type position is that class, so a value of
        // another type is not one and an instance of it is
        let errs = analyze_entry("local a: RequireType(\"m\").Foo = 1", &p);
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
        let ok = analyze_entry(
            "local a: RequireType(\"m\").Foo = RequireType(\"m\").Foo.new()\nreturn a.x\n",
            &p,
        );
        assert!(ok.is_empty(), "got: {ok:?}");
    }

    #[test]
    fn a_class_from_another_module_keeps_its_fields() {
        let p = provider(&[("m", "export object Point\n    x: int\nend")]);
        // `Point.new()` is a `Point` and reading a field off it is a number: the
        // class crossed as itself rather than as `any`
        let ok = analyze_entry(
            "local m = require(\"m\")\nlocal p = m.Point.new()\nlocal n: int = p.x\nreturn n\n",
            &p,
        );
        assert!(ok.is_empty(), "got: {ok:?}");
        let errs = analyze_entry(
            "local m = require(\"m\")\nlocal p = m.Point.new()\nlocal s: string = p.x\nreturn s\n",
            &p,
        );
        assert!(errs.iter().any(|e| e.contains("Type")), "got: {errs:?}");
    }

    #[test]
    fn a_class_of_the_same_name_in_two_modules_is_two_classes() {
        let p = provider(&[
            ("a", "export object Point\n    x: int\nend"),
            ("b", "export object Point\n    x: int\nend"),
        ]);
        // both are called `Point` and both are id `0` in their own tables, which
        // is exactly why a class says which module declared it
        let errs = analyze_entry(
            "local a = require(\"a\")\nlocal b = require(\"b\")\n\
             local p: a.Point = b.Point.new()\nreturn p\n",
            &p,
        );
        assert!(
            errs.iter().any(|e| e.contains("Type")),
            "a class in another module is only ever itself: {errs:?}"
        );
    }

    #[test]
    fn require_type_circular_errors() {
        let p = provider(&[
            ("a", "export type X = RequireType(\"b\").Y"),
            ("b", "export type Y = RequireType(\"a\").X"),
        ]);
        let errs = analyze_entry("local t: RequireType(\"a\").X", &p);
        assert!(
            errs.iter().any(|e| e.contains("circular require")),
            "got: {errs:?}"
        );
    }

    #[test]
    fn require_type_missing_module_is_any() {
        let p = provider(&[]);
        let errs = analyze_entry("local a: RequireType(\"missing\").X = 1", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn a_declared_export_is_callable_with_what_it_promised() {
        let p = provider(&[("m", "@declare export function twice(a: int, b: int): int")]);
        let errs = analyze_entry("local m = require(\"m\") local n: int = m.twice(1, 2)", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn a_declared_export_is_judged_by_what_it_promised() {
        let p = provider(&[("m", "@declare export function twice(a: int, b: int): int")]);
        let errs = analyze_entry("local m = require(\"m\") local n: int = m.twice(1)", &p);
        assert!(
            errs.iter().any(|e| e.contains("arguments")),
            "the declaration is the signature, and the call is judged against it: {errs:?}"
        );
    }

    #[test]
    fn a_declared_value_reaches_the_importer_as_the_type_it_was_given() {
        let p = provider(&[("m", "@declare export global HOSTNAME: string")]);
        let ok = analyze_entry("local m = require(\"m\") local s: string = m.HOSTNAME", &p);
        assert!(ok.is_empty(), "got: {ok:?}");
        let errs = analyze_entry("local m = require(\"m\") local n: int = m.HOSTNAME", &p);
        assert!(
            errs.iter().any(|e| e.contains("Type")),
            "a declared value has the type it was annotated with, not `any`: {errs:?}"
        );
    }

    #[test]
    fn a_require_type_asks_a_declaration_for_the_shape_it_promised() {
        let p = provider(&[("m", "@declare export function twice(a: int): int")]);
        let ok = analyze_entry(
            "local function f(a: int) return 1 end\n\
             local g: RequireType(\"m\").twice = f\n",
            &p,
        );
        assert!(ok.is_empty(), "got: {ok:?}");
        let errs = analyze_entry("local g: RequireType(\"m\").twice = 1", &p);
        assert!(
            errs.iter().any(|e| e.contains("Type")),
            "there is nothing to evaluate on the other side, but there is a shape: {errs:?}"
        );
    }

    #[test]
    fn a_require_type_of_anything_else_is_still_any() {
        // only a declaration answers from its value: an ordinary export has a
        // body, and what a type position means by it is the type it exports
        let p = provider(&[("m", "export function twice(a) return 1 end")]);
        let errs = analyze_entry("local g: RequireType(\"m\").twice = \"s\"", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    #[test]
    fn a_declared_export_is_told_apart_from_a_built_one() {
        let p = provider(&[(
            "m",
            "@declare export function host(): int\nexport function mine(): int return 1 end",
        )]);
        let modules = build_modules("local m = require(\"m\")", &p);
        let module = modules.get("m:m").expect("the module was loaded");
        assert!(module.value_is_declared("host"), "a declaration says so");
        assert!(
            !module.value_is_declared("mine"),
            "a function with a body is a value, however it was written"
        );
        assert!(
            !module.value_is_declared("nothing"),
            "and an unknown name says nothing"
        );
    }
}
