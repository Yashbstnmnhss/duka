use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use duka_lib::duka_frontend::{
    analyzer::{
        ScopeAnalysis, Visit, Visitor,
        modules::{ModuleMap, ModuleType},
    },
    lexer::token::{Token, TokenKind},
    parser::ast::{DukaChunk, Expr, Node, NodeKind, Path, PathSuffix},
};
use duka_lib::duka_shared::{
    constants::ctype,
    errors::{
        DeprecatedUseLint, DukaDiagnose, DukaErrorLevel, DukaSemanticError, DukaSpannedError,
        MustUseLint, Span,
    },
    types::SourceInfo,
};

use crate::compile::ModuleResolver;

pub fn collect(
    chunk: &DukaChunk,
    scope: &ScopeAnalysis,
    tokens: &[Token],
    text: &str,
    resolver: &ModuleResolver,
    modules: &ModuleMap,
) -> Vec<DukaDiagnose> {
    let source = Arc::new(chunk.source_info.clone());
    let bindings = require_bindings(tokens, resolver, modules);
    let mut reported: HashSet<Span> = HashSet::new();
    let mut out: Vec<DukaDiagnose> = vec![];

    for (span, id) in &scope.uses {
        if reported.contains(span) {
            continue;
        }
        let Some(symbol) = scope.symbols.symbol_by_id(*id) else {
            continue;
        };
        let Some(info) = symbol.attributes.deprecated() else {
            continue;
        };
        let name = token_name(tokens, *span).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        reported.insert(*span);
        warn(
            DukaSemanticError::DeprecatedUse(DeprecatedUseLint {
                name: name.into(),
                reason: info.reason.into(),
                replacement: info.replacement.into(),
            }),
            *span,
            &source,
            &mut out,
        );
    }

    for (span, path) in &scope.member_paths {
        if reported.contains(span) {
            continue;
        }
        let Some((base, member)) = path.rsplit_once('.') else {
            continue;
        };
        let Some(module) = module_for_base(&bindings, resolver, modules, base) else {
            continue;
        };
        let Some(info) = module.value_attributes(member).and_then(|attrs| {
            attrs
                .deprecated()
                .map(|info| (info.reason.to_owned(), info.replacement.to_owned()))
        }) else {
            continue;
        };
        reported.insert(*span);
        warn(
            DukaSemanticError::DeprecatedUse(DeprecatedUseLint {
                name: member.into(),
                reason: info.0.into(),
                replacement: info.1.into(),
            }),
            *span,
            &source,
            &mut out,
        );
    }

    let mut visitor = MustUse {
        scope,
        bindings: &bindings,
        resolver,
        modules,
        text,
        reported: &mut reported,
        source: Arc::clone(&source),
        out: &mut out,
    };
    chunk.visit(&mut visitor);
    out
}

fn warn(
    kind: DukaSemanticError,
    span: Span,
    source: &Arc<SourceInfo>,
    out: &mut Vec<DukaDiagnose>,
) {
    out.push(DukaDiagnose::from(DukaSpannedError {
        kind: kind.into(),
        span,
        level: DukaErrorLevel::Warning,
        source_info: Arc::clone(source),
        related: Box::new([]),
    }));
}

fn token_name(tokens: &[Token], span: Span) -> Option<&str> {
    tokens
        .iter()
        .find(|(_, at)| *at == span)
        .and_then(|(kind, _)| match kind {
            TokenKind::Ident(name) => Some(&**name),
            _ => None,
        })
}

fn module_for_base<'a>(
    bindings: &HashMap<String, &'a ModuleType>,
    resolver: &ModuleResolver,
    modules: &'a ModuleMap,
    base: &str,
) -> Option<&'a ModuleType> {
    bindings
        .get(base)
        .copied()
        .or_else(|| resolver.resolve(modules, base))
}

fn require_bindings<'a>(
    tokens: &[Token],
    resolver: &ModuleResolver,
    modules: &'a ModuleMap,
) -> HashMap<String, &'a ModuleType> {
    let mut out = HashMap::new();
    for i in 0..tokens.len() {
        let TokenKind::Ident(name) = &tokens[i].0 else {
            continue;
        };
        if !matches!(
            tokens.get(i + 1).map(|(kind, _)| kind),
            Some(TokenKind::Assign)
        ) {
            continue;
        }
        let mut at = i + 2;
        while tokens.get(at).is_some_and(|(kind, _)| kind.is_comment()) {
            at += 1;
        }
        let Some((TokenKind::Ident(callee), _)) = tokens.get(at) else {
            continue;
        };
        let mut argument = at + 1;
        let parenthesised = matches!(
            tokens.get(argument).map(|(kind, _)| kind),
            Some(TokenKind::LParen)
        );
        if parenthesised {
            argument += 1;
        }
        let Some(TokenKind::String(bytes)) = tokens.get(argument).map(|(kind, _)| kind) else {
            continue;
        };
        if !(&**callee == "require" || (parenthesised && &**callee == ctype::REQUIRE)) {
            continue;
        }
        let module = String::from_utf8_lossy(bytes);
        if let Some(module) = resolver.resolve(modules, &module) {
            out.insert(name.to_string(), module);
        }
    }
    out
}

struct MustUse<'a> {
    scope: &'a ScopeAnalysis,
    bindings: &'a HashMap<String, &'a ModuleType>,
    resolver: &'a ModuleResolver,
    modules: &'a ModuleMap,
    text: &'a str,
    reported: &'a mut HashSet<Span>,
    source: Arc<SourceInfo>,
    out: &'a mut Vec<DukaDiagnose>,
}

impl MustUse<'_> {
    fn target(&self, callee: &Node) -> Option<(String, String)> {
        let NodeKind::Access(path) = &callee.0 else {
            return None;
        };
        let (name, span, member) = path_name(path)?;
        if member {
            let path = self.scope.member_paths.get(&span)?;
            let (base, member) = path.rsplit_once('.')?;
            let module = module_for_base(self.bindings, self.resolver, self.modules, base)?;
            let reason = module
                .value_attributes(member)?
                .must_use_reason()?
                .to_owned();
            return Some((member.to_owned(), reason));
        }
        let symbol = self.scope.symbols.symbol_at_span(span).or_else(|| {
            self.scope
                .uses
                .get(&span)
                .and_then(|id| self.scope.symbols.symbol_by_id(*id))
        })?;
        let reason = symbol.attributes.must_use_reason()?.to_owned();
        Some((name, reason))
    }
}

impl Visitor for MustUse<'_> {
    fn visit_node(&mut self, expr: &Expr) {
        let NodeKind::Call(callee, _) = &expr.0 else {
            return;
        };
        if !is_discarded(self.text, expr.1) {
            return;
        }
        let Some((name, reason)) = self.target(callee) else {
            return;
        };
        let span = expr.1;
        if !self.reported.insert(span) {
            return;
        }
        warn(
            DukaSemanticError::MustUseDiscarded(MustUseLint {
                name: name.into(),
                reason: reason.into(),
            }),
            span,
            &self.source,
            self.out,
        );
    }
}

fn is_discarded(text: &str, span: Span) -> bool {
    let prefix = line_prefix(text, span.start.line, span.start.column);
    if !prefix.trim().is_empty() {
        return false;
    }
    let suffix = line_suffix(text, span.end.line, span.end.column);
    matches!(suffix.trim(), "" | ";")
}

fn line_prefix(text: &str, line: u32, column: u32) -> &str {
    let Some(line) = text.lines().nth(line.saturating_sub(1) as usize) else {
        return "";
    };
    &line[..column_offset(line, column)]
}

fn line_suffix(text: &str, line: u32, column: u32) -> &str {
    let Some(line) = text.lines().nth(line.saturating_sub(1) as usize) else {
        return "";
    };
    &line[column_offset(line, column)..]
}

fn column_offset(line: &str, column: u32) -> usize {
    line.char_indices()
        .nth(column.saturating_sub(1) as usize)
        .map(|(at, _)| at)
        .unwrap_or(line.len())
}

fn path_name(path: &Path) -> Option<(String, Span, bool)> {
    match path {
        Path::Base((name, span)) => Some((name.to_string(), *span, false)),
        Path::Chain(_, PathSuffix::Dot((name, span)) | PathSuffix::Colon((name, span))) => {
            Some((name.to_string(), *span, true))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use duka_lib::duka_shared::errors::Severity;

    use super::*;
    use crate::compile;

    fn analyze(src: &str) -> compile::DocAnalysis {
        let mut cache = duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache::default();
        compile::analyze(src, "test.duka", None, &HashMap::new(), &mut cache)
    }

    fn warnings(analysis: &compile::DocAnalysis) -> Vec<&DukaSemanticError> {
        analysis
            .errors
            .iter()
            .filter(|error| error.severity() == Severity::Warning)
            .filter_map(|error| match error.kind() {
                duka_lib::duka_shared::errors::DukaErrorKind::Semantic(error) => Some(error),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_deprecated_call_is_a_warning() {
        let src = concat!(
            "@deprecated(reason = \"old\", use = \"new_fn\")\n",
            "function old_fn(): int\n",
            "    return 1\n",
            "end\n",
            "local x = old_fn()\n",
            "return x\n",
        );
        let analysis = analyze(src);
        let warnings = warnings(&analysis);
        assert_eq!(warnings.len(), 1, "{:?}", analysis.errors);
        let DukaSemanticError::DeprecatedUse(lint) = warnings[0] else {
            panic!("expected a deprecation: {:?}", warnings[0]);
        };
        assert_eq!(&*lint.name, "old_fn");
        assert_eq!(&*lint.replacement, "new_fn");
        assert!(lint.reason.contains("old"), "{:?}", lint.reason);
        assert!(warnings[0].get_help().contains("new_fn"));
    }

    #[test]
    fn a_deprecated_declaration_alone_is_not_a_warning() {
        let src = concat!(
            "@deprecated(reason = \"old\")\n",
            "function old_fn(): int\n",
            "    return 1\n",
            "end\n",
            "return 1\n",
        );
        let analysis = analyze(src);
        assert!(warnings(&analysis).is_empty(), "{:?}", analysis.errors);
    }

    #[test]
    fn a_discarded_must_use_result_is_a_warning() {
        let src = concat!(
            "@must_use(reason = \"check the result\")\n",
            "function f(): int\n",
            "    return 1\n",
            "end\n",
            "f()\n",
            "local x = f()\n",
            "return x\n",
        );
        let analysis = analyze(src);
        let warnings = warnings(&analysis);
        assert_eq!(warnings.len(), 1, "{:?}", analysis.errors);
        let DukaSemanticError::MustUseDiscarded(lint) = warnings[0] else {
            panic!("expected must_use: {:?}", warnings[0]);
        };
        assert_eq!(&*lint.name, "f");
        assert!(warnings[0].get_help().contains("local _ ="));
    }

    #[test]
    fn a_deprecated_module_member_is_a_warning() {
        let dir = std::env::temp_dir().join("duka-lsp-lint-module");
        let module = concat!(
            "@deprecated(reason = \"old\", use = \"new_fn\")\n",
            "export function old_fn(): int\n",
            "    return 1\n",
            "end\n",
            "@must_use(reason = \"check it\")\n",
            "export function f(): int\n",
            "    return 2\n",
            "end\n",
        );
        let mut open = HashMap::new();
        open.insert(
            dir.join("modules").join("host.duka"),
            std::sync::Arc::new(module.to_owned()),
        );
        let text = concat!(
            "local host = require(\"host\")\n",
            "local a = host.old_fn()\n",
            "host.f()\n",
            "return a\n",
        );
        let mut cache = duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache::default();
        let analysis = compile::analyze(
            text,
            "main.duka",
            Some(&dir.join("main.duka")),
            &open,
            &mut cache,
        );
        let warnings = warnings(&analysis);
        assert_eq!(warnings.len(), 2, "{:?}", analysis.errors);
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            DukaSemanticError::DeprecatedUse(lint) if &*lint.name == "old_fn"
        )));
        assert!(warnings.iter().any(|warning| matches!(
            warning,
            DukaSemanticError::MustUseDiscarded(lint) if &*lint.name == "f"
        )));
    }

    #[test]
    fn a_used_must_use_result_is_not_a_warning() {
        let src = concat!(
            "@must_use(reason = \"check the result\")\n",
            "function f(): int\n",
            "    return 1\n",
            "end\n",
            "local x = f()\n",
            "return x\n",
        );
        let analysis = analyze(src);
        assert!(warnings(&analysis).is_empty(), "{:?}", analysis.errors);
    }
}
