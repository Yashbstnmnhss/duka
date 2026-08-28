use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;

use duka_shared::constants::ctype;
use duka_shared::{
    config::{DukaAnalyzerConfig, DukaLexerConfig, DukaParserConfig},
    dtype::Type,
    errors::{DukaSemanticError, DukaSpannedError, Span},
    types::{DukaAnalyzer, DukaLexer, DukaParser, SourceInfo},
    utils::{SymbolTableViewer, SymbolType},
    value::ConstValue,
};

use crate::{
    analyzer::{AnalyzerData, ScopeAnalysis, ScopeAnalyzer, Visit, Visitor},
    lexer::LexerWithMacro,
    parser::{
        Parser,
        ast::{
            Block, DukaChunk, Expr, ExprKind, Field, FuncBody, If, IfClause, Linq, LinqClause,
            Match, ObjectDef, ObjectProperty, Param, Path, PathSuffix, Pattern, PatternTerm, Stmt,
            StmtKind, TypeDescriptor,
        },
    },
};

pub trait DukaSourceProvider {
    fn load(&self, name: &str, caller_path: Option<&str>) -> Option<(Box<str>, Arc<[u8]>)>;
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

/// 指纹算法
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 模块构建缓存: 依赖源码指纹未变时整体复用上次的分析结果
#[derive(Default)]
pub struct ModuleBuildCache {
    fingerprint: Vec<(Box<str>, u64)>,
    modules: ModuleMap,
    refs: HashMap<Box<str>, Vec<(String, Span)>>,
}

fn walk_fingerprint(
    ref_names: &[(String, Span)],
    caller_path: Option<&str>,
    provider: &dyn DukaSourceProvider,
    cache_refs: &HashMap<Box<str>, Vec<(String, Span)>>,
    cache_modules: &ModuleMap,
    out: &mut Vec<(Box<str>, u64)>,
    visited: &mut HashSet<Box<str>>,
) -> bool {
    for (name, _) in ref_names {
        let Some((key, bytes)) = provider.load(name, caller_path) else {
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
            Some(key.as_ref()),
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
    let entry_key: Box<str> = entry
        .source_info
        .name
        .as_deref()
        .map(Box::from)
        .unwrap_or_else(|| Box::from("<entry>"));
    let entry_refs = collect_refs(entry);

    if !cache.modules.is_empty() && !cache.fingerprint.is_empty() {
        let mut fp = vec![];
        let mut visited = HashSet::new();
        if walk_fingerprint(
            &entry_refs,
            entry.source_info.name.as_deref(),
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
        },
    );
    for (name, span) in collect_refs(entry) {
        collect_module(
            &name,
            entry.source_info.name.as_deref(),
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
        entry.source_info.name.as_deref(),
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
    caller_path: Option<&str>,
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
    let Some((key, src)) = provider.load(name, caller_path) else {
        return;
    };
    if modules.contains_key(&key) {
        return;
    }
    if !loading.insert(key.clone()) {
        errors.push(DukaSpannedError {
            kind: DukaSemanticError::CircularRequire(key.clone()).into(),
            span,
            related: [].into(),
            source_info: Arc::new(SourceInfo {
                name: Some(Arc::from(key.as_ref())),
                source: Arc::new([]),
                time: duka_shared::types::current_debug_time(),
            }),
        });
        return;
    }
    let source = String::from_utf8_lossy(&src).into_owned();
    let lexer = LexerWithMacro::new(
        Cursor::new(source.as_str()),
        Some(key.to_string()),
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
    let chunk = match Parser::parse(stream, parser_cfg.clone()) {
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
            Some(&key),
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
    let m = ModuleType {
        key: key.clone(),
        source: Arc::new(chunk.source_info.clone()),
        analysis: Arc::new(data.1),
        exported,
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
    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(stmt, self.out);
    }
    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(expr, self.out);
    }
}

fn walk_stmt(stmt: &Stmt, out: &mut Vec<(String, Span)>) {
    match &stmt.0 {
        StmtKind::TypeAlias(_, ty) => walk_type_value(ty, out),
        StmtKind::TypeFunction(_, body) => walk_func_body(body, out),
        StmtKind::Object(obj) => walk_object(obj, out),
        StmtKind::Define(attrs, exprs, _) => {
            for attr in attrs.iter() {
                if let Some(ty) = &attr.0.2 {
                    walk_type_value(ty, out);
                }
            }
            for e in exprs.iter() {
                walk_expr(e, out);
            }
        }
        StmtKind::Function(_, _, body, _) => walk_func_body(body, out),
        StmtKind::Export(inner) => walk_stmt(inner, out),
        StmtKind::Assign(_, exprs) => {
            for e in exprs.iter() {
                walk_expr(e, out);
            }
        }
        StmtKind::While(cond, body) => {
            walk_expr(cond, out);
            walk_block(body, out);
        }
        StmtKind::Do(body) => walk_block(body, out),
        StmtKind::ForNumeric(_, start, limit, step, body) => {
            walk_expr(start, out);
            walk_expr(limit, out);
            if let Some(step) = step {
                walk_expr(step, out);
            }
            walk_block(body, out);
        }
        StmtKind::ForGeneric(_, exprs, body) => {
            for e in exprs.iter() {
                walk_expr(e, out);
            }
            walk_block(body, out);
        }
        StmtKind::Match(m) => walk_match(m, out),
        StmtKind::Return(exprs) => {
            for e in exprs.iter() {
                walk_expr(e, out);
            }
        }
        StmtKind::If(if_) => walk_if(if_, out),
        _ => {}
    }
}

fn walk_block(block: &Block, out: &mut Vec<(String, Span)>) {
    for s in block.0.iter() {
        walk_stmt(s, out);
    }
    if let Some(last) = &block.1 {
        walk_stmt(last, out);
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
    walk_block(&body.3, out);
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

fn walk_pattern(p: &Pattern, out: &mut Vec<(String, Span)>) {
    match &p.0 {
        PatternTerm::Bind(_, ty) => {
            if let Some(ty) = ty {
                walk_type_value(ty, out);
            }
        }
        PatternTerm::Call(e) => walk_expr(e, out),
        PatternTerm::Compare(_, e) => walk_expr(e, out),
        _ => {}
    }
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
        ExprKind::Call(f, args) => {
            if let ExprKind::Access(p) = &f.0
                && let Path::Base((name, _)) = p.as_ref()
                && name.as_str() == "require"
                && let Some(Expr(ExprKind::Literal(ConstValue::String(bytes)), _)) = args.first()
            {
                out.push((String::from_utf8_lossy(bytes).into_owned(), expr.1));
            }
            walk_expr(f, out);
            for a in args.iter() {
                walk_expr(a, out);
            }
        }
        ExprKind::Access(path) => walk_path(path, out),
        ExprKind::Do(block) => walk_block(block, out),
        ExprKind::Table(fields) => {
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
        ExprKind::Array(items) => {
            for e in items.iter() {
                walk_expr(e, out);
            }
        }
        ExprKind::Function(body) => walk_func_body(body, out),
        ExprKind::Unary(e, _) => walk_expr(e, out),
        ExprKind::Binary(a, b, _) => {
            walk_expr(a, out);
            walk_expr(b, out);
        }
        ExprKind::If(if_) => walk_if(if_, out),
        ExprKind::Match(m) => walk_match(m, out),
        ExprKind::Linq(l) => walk_linq(l, out),
        ExprKind::TypeLit(tv) => walk_type_value(tv, out),
        ExprKind::Empty
        | ExprKind::VarArg
        | ExprKind::Literal(_)
        | ExprKind::SysCall(_)
        | ExprKind::BangMacro(_) => {}
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

fn walk_type_value(tv: &TypeDescriptor, out: &mut Vec<(String, Span)>) {
    match tv {
        TypeDescriptor::TypeCall { name, args, span } => {
            if name.as_ref() == ctype::REQUIRE
                && let Some(TypeDescriptor::Pure(Type::Literal(ConstValue::String(bytes)))) =
                    args.first()
            {
                out.push((String::from_utf8_lossy(bytes).into_owned(), *span));
            }
            for a in args.iter() {
                walk_type_value(a, out);
            }
        }
        TypeDescriptor::Access {
            base,
            member: _,
            args,
            span: _,
        } => {
            if let Some(args) = args {
                for a in args.iter() {
                    walk_type_value(a, out);
                }
            }
            walk_type_value(base, out);
        }
        TypeDescriptor::TypeOf { expr, .. } => walk_expr(expr, out),
        TypeDescriptor::Generic { args, .. } => {
            for a in args.iter() {
                walk_type_value(a, out);
            }
        }
        TypeDescriptor::Array(e) => {
            if let Some(e) = e {
                walk_type_value(e, out);
            }
        }
        TypeDescriptor::Table(k, v) => {
            if let Some(k) = k {
                walk_type_value(k, out);
            }
            if let Some(v) = v {
                walk_type_value(v, out);
            }
        }
        TypeDescriptor::Union(ts) => {
            for t in ts.iter() {
                walk_type_value(t, out);
            }
        }
        TypeDescriptor::TypeTuple(ts) => {
            for t in ts.iter() {
                walk_type_value(t, out);
            }
        }
        TypeDescriptor::TypeTable(ts) => {
            for (_, v) in ts.iter() {
                walk_type_value(v, out);
            }
        }
        TypeDescriptor::Function(ft) => {
            if let Some(ft) = ft {
                for p in ft.params.iter() {
                    walk_type_value(p, out);
                }
                for r in ft.returns.iter() {
                    walk_type_value(r, out);
                }
            }
        }
        TypeDescriptor::Pure(_)
        | TypeDescriptor::FnLit(..)
        | TypeDescriptor::NonNil(_)
        | TypeDescriptor::Nilable(_)
        | TypeDescriptor::Named(..)
        | TypeDescriptor::Rec(_) => {}
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
    if let Some(last) = &block.1 {
        collect_exports_stmt(last, viewer, exported);
    }
}

fn collect_exports_stmt(
    stmt: &Stmt,
    viewer: &SymbolTableViewer<'_>,
    exported: &mut HashMap<Box<str>, ExportedTypeKind>,
) {
    let StmtKind::Export(inner) = &stmt.0 else {
        return;
    };
    match &inner.0 {
        StmtKind::TypeAlias((name, _), _) => {
            if let Some(sym) = viewer.lookup(name) {
                if let SymbolType::TypeAlias(id) = sym.symbol_type {
                    exported.insert(name.clone().into_boxed_str(), ExportedTypeKind::Alias(id));
                }
            }
        }
        StmtKind::TypeFunction((name, _), _) => {
            if let Some(sym) = viewer.lookup(name) {
                if let SymbolType::TypeFunction(id) = sym.symbol_type {
                    exported.insert(name.clone().into_boxed_str(), ExportedTypeKind::TypeFn(id));
                }
            }
        }
        StmtKind::InlineTypeFunction((name, _), _, _) => {
            if let Some(sym) = viewer.lookup(name) {
                if let SymbolType::InlineTypeFunction(id) = sym.symbol_type {
                    exported.insert(
                        name.clone().into_boxed_str(),
                        ExportedTypeKind::InlineFn(id),
                    );
                }
            }
        }
        StmtKind::Object(obj) => {
            if let Some(sym) = viewer.lookup(&obj.name.0) {
                if let SymbolType::ObjectClass(id) = sym.symbol_type {
                    exported.insert(
                        obj.name.0.clone().into_boxed_str(),
                        ExportedTypeKind::Object(id),
                    );
                }
            }
        }
        _ => {}
    }
}

/// 让Object都变Any
pub fn sanitize_foreign(t: Type) -> Type {
    match t {
        Type::Object { .. } => Type::Any,
        Type::Array(Some(inner)) => Type::Array(Some(Box::new(sanitize_foreign(*inner)))),
        Type::Array(None) => Type::Array(None),
        Type::Table(k, v) => Type::Table(
            k.map(|k| Box::new(sanitize_foreign(*k))),
            v.map(|v| Box::new(sanitize_foreign(*v))),
        ),
        Type::Union(ts) => Type::Union(ts.into_vec().into_iter().map(sanitize_foreign).collect()),
        Type::TypeTuple(ts) => Type::TypeTuple(ts.into_iter().map(sanitize_foreign).collect()),
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
    caller_path: Option<&str>,
    provider: &dyn DukaSourceProvider,
) -> Option<&'a ModuleType> {
    if let Some(m) = modules.get(name) {
        return Some(m);
    }
    let (key, _) = provider.load(name, caller_path)?;
    modules.get(&key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::analyzer::{TypeChecker, TypeEval};
    use duka_shared::config::{DukaAnalyzerConfig, DukaLexerConfig, DukaParserConfig};

    struct TestProvider {
        modules: HashMap<String, String>,
    }

    impl DukaSourceProvider for TestProvider {
        fn load(&self, name: &str, _caller_path: Option<&str>) -> Option<(Box<str>, Arc<[u8]>)> {
            let src = self.modules.get(name)?;
            Some((
                format!("m:{name}").into_boxed_str(),
                src.as_bytes().to_vec().into(),
            ))
        }
    }

    fn analyze_entry(entry: &str, provider: &TestProvider) -> Vec<String> {
        let lexer = LexerWithMacro::new(
            Cursor::new(entry),
            Some("main.duka".to_owned()),
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
    fn require_type_alias_accepts_match() {
        let p = provider(&[("m", "export type Alias = { x: int, y: string }")]);
        let errs = analyze_entry(
            "local a: RequireType(\"m\").Alias = { x = 1, y = \"s\" }",
            &p,
        );
        assert!(errs.is_empty(), "got: {errs:?}");
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
    fn require_type_object_is_any() {
        let p = provider(&[("m", "export object Foo\n    x = 1\nend")]);
        let errs = analyze_entry("local a: RequireType(\"m\").Foo = 1", &p);
        assert!(errs.is_empty(), "got: {errs:?}");
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
}
