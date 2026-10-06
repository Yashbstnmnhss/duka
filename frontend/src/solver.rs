//! Constraint solving for generic type parameters
//!
//! The solver only knows about [`Type`] and a set of declared variables, it has
//! no notion of functions, scopes or modules, so it can be reasoned about (and
//! tested) on its own.

use std::collections::HashMap;

use duka_shared::dtype::{FunctionType, Type};
use duka_shared::errors::Span;

use crate::parser::ast::ParamShape;

/// A type variable declared by a generic signature
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub name: Box<str>,
    /// `T: bound`, the bound may mention other variables
    pub bound: Option<Type>,
    /// `T = default`, used when nothing else determines the variable
    pub default: Option<Type>,
    pub span: Span,
    /// `...Ts` takes the remaining type arguments as a list rather than one type
    pub shape: ParamShape,
}

/// `T = candidate`, produced by pairing a parameter type with an argument type
#[derive(Debug, Clone)]
struct Equality {
    var: Box<str>,
    candidate: Type,
    span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Diagnostic {
    /// a variable collected two candidates that do not agree
    Ambiguous {
        name: Box<str>,
        first: Type,
        second: Type,
        span: Span,
    },
    /// a candidate does not satisfy the declared bound
    BoundViolated {
        name: Box<str>,
        bound: Type,
        candidate: Type,
        span: Span,
    },
    /// explicit type arguments do not line up with the declaration
    ArityMismatch {
        expected: usize,
        given: usize,
        span: Span,
    },
}

#[derive(Debug, Clone, Default)]
pub struct Solution {
    pub bindings: HashMap<Box<str>, Type>,
    pub diagnostics: Vec<Diagnostic>,
    /// the call this solution came from, so a diagnostic can point at the call
    /// rather than at the type parameter's declaration
    pub span: Span,
}

/// Collects constraints for one generic signature and solves them
#[derive(Debug, Default)]
pub struct Solver {
    vars: Vec<VarDecl>,
    equalities: Vec<Equality>,
    solution: Solution,
}

impl Solver {
    pub fn new(vars: Vec<VarDecl>) -> Self {
        Self {
            vars,
            equalities: vec![],
            solution: Solution::default(),
        }
    }

    /// Ties a declared parameter list to the argument types of a call
    pub fn constrain_call(&mut self, formals: &[Type], actuals: &[Type], span: Span) {
        for (formal, actual) in formals.iter().zip(actuals.iter()) {
            self.collect(formal, actual, span);
        }
    }

    /// Solves the collected constraints, `given` are the explicit type arguments
    /// and take precedence over anything the call site implies
    pub fn solve(&mut self, given: Vec<Type>, call_span: Span) -> &Solution {
        self.solution.span = call_span;
        let inferred = given.is_empty();
        if !given.is_empty() {
            // A pack takes whatever is left over, so the count that has to line
            // up is the count of the fixed parameters before it. Zero for a
            // signature that is nothing but a pack, which is legal and means "all
            // of the type arguments, as a list".
            let fixed = self
                .vars
                .iter()
                .filter(|v| v.shape == ParamShape::Fixed)
                .count();
            if given.len() < fixed || given.len() > self.vars.len() {
                self.solution.diagnostics.push(Diagnostic::ArityMismatch {
                    expected: fixed,
                    given: given.len(),
                    span: call_span,
                });
            } else {
                let mut rest = given.iter();
                for var in &self.vars {
                    // The pack is last, so everything not yet taken is its list.
                    // The list is a `Type::TypeTuple` like any other, which is
                    // what lets `Ts` be indexed, concatenated and counted with
                    // the operations that already work on tuples.
                    let ty = match var.shape {
                        ParamShape::Fixed => match rest.next() {
                            Some(ty) => ty.clone(),
                            None => Type::Any,
                        },
                        ParamShape::Pack => Type::TypeTuple(rest.by_ref().cloned().collect()),
                    };
                    self.solution.bindings.insert(var.name.clone(), ty);
                }
            }
        }
        if self.solution.diagnostics.is_empty() {
            // explicit type arguments replace inference, only the declared
            // bounds are still checked against them
            if inferred {
                for _ in 0..=self.vars.len() {
                    let mut changed = self.solve_equalities();
                    changed |= self.solve_bounds_forward();
                    changed |= self.solve_bounds_inverse();
                    if !changed {
                        break;
                    }
                }
            }
            self.check_bounds();
        }
        // a default may mention other variables (`U = array<T>`), so it is
        // substituted with whatever is known by the time we reach it, which is
        // why the variables are finalized in declaration order
        for var in &self.vars {
            if !self.solution.bindings.contains_key(&var.name) {
                let fallback = match &var.default {
                    Some(default) => self.substitute(default),
                    None => Type::Any,
                };
                self.solution.bindings.insert(var.name.clone(), fallback);
            }
        }
        &self.solution
    }

    /// Replaces every bound variable inside a type
    ///
    /// The bindings are all this needs and nothing else, so it lives on the
    /// solution rather than on the solver. That is what lets a caller hold the
    /// solution while reading a signature: `solve` hands back a borrow of the
    /// solver, so the solver cannot be lent out a second time at the same time.
    pub fn substitute(&self, ty: &Type) -> Type {
        self.solution.substitute(ty)
    }

    fn is_var(&self, name: &str) -> bool {
        self.vars.iter().any(|v| v.name.as_ref() == name)
    }

    /// Whether a type carries enough information to constrain a variable, an
    /// opaque argument can neither infer nor be checked against a formal type
    pub fn has_info(ty: &Type) -> bool {
        match ty {
            Type::Any | Type::Nil | Type::Never | Type::Array(None) => false,
            Type::Table(None, None) => false,
            _ => true,
        }
    }

    /// Whether a type mentions any declared variable
    fn mentions(&self, ty: &Type) -> bool {
        match ty {
            Type::Param(name) => self.is_var(name),
            Type::Array(inner) => inner.as_deref().is_some_and(|t| self.mentions(t)),
            Type::Table(k, v) => {
                k.as_deref().is_some_and(|t| self.mentions(t))
                    || v.as_deref().is_some_and(|t| self.mentions(t))
            }
            Type::Union(ts) => ts.iter().any(|t| self.mentions(t)),
            Type::TypeTuple(ts) => ts.iter().any(|t| self.mentions(t)),
            Type::TypeTable(fields) => fields.iter().any(|(_, t)| self.mentions(t)),
            Type::Object { args, .. } => args.iter().any(|t| self.mentions(t)),
            Type::Function(Some(ft)) => ft
                .params
                .iter()
                .chain(ft.returns.iter())
                .any(|t| self.mentions(t)),
            Type::Rec(inner) => self.mentions(inner),
            _ => false,
        }
    }

    /// Pairs a declared parameter type with an argument type and records every
    /// `T = candidate` it implies. Shapes that do not line up are skipped on
    /// purpose: the caller reports them when it checks the argument itself, so a
    /// mismatch is reported exactly once.
    fn collect(&mut self, formal: &Type, actual: &Type, span: Span) {
        if !Self::has_info(actual) {
            return;
        }
        match formal {
            Type::Param(name) if self.is_var(name) => {
                self.equalities.push(Equality {
                    var: name.clone(),
                    candidate: actual.clone(),
                    span,
                });
            }
            Type::Array(formal_inner) => match actual {
                Type::Array(Some(actual_inner)) => {
                    if let Some(formal_inner) = formal_inner {
                        self.collect(formal_inner, actual_inner, span);
                    }
                }
                Type::TypeTuple(actuals) => {
                    if let Some(formal_inner) = formal_inner {
                        for actual in actuals.clone() {
                            self.collect(formal_inner, &actual, span);
                        }
                    }
                }
                _ => {}
            },
            Type::Table(formal_k, formal_v) => {
                let key_info = formal_k.as_deref().is_some_and(Self::has_info);
                let value_info = formal_v.as_deref().is_some_and(Self::has_info);
                match actual {
                    Type::Table(actual_k, actual_v) => {
                        if key_info
                            && let (Some(fk), Some(ak)) = (formal_k.as_deref(), actual_k.as_deref())
                        {
                            self.collect(fk, ak, span);
                        }
                        if value_info
                            && let (Some(fv), Some(av)) = (formal_v.as_deref(), actual_v.as_deref())
                        {
                            self.collect(fv, av, span);
                        }
                    }
                    Type::TypeTable(fields) => {
                        if let Some(fv) = formal_v.as_deref() {
                            for (_, actual) in fields.iter() {
                                self.collect(fv, actual, span);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Type::TypeTable(formals) => {
                if let Type::TypeTable(actuals) = actual {
                    for (key, formal) in formals.iter() {
                        if let Some((_, actual)) = actuals.iter().find(|(k, _)| k == key) {
                            self.collect(formal, actual, span);
                        }
                    }
                }
            }
            Type::TypeTuple(formals) => match actual {
                Type::TypeTuple(actuals) => {
                    for (formal, actual) in formals.iter().zip(actuals.iter()) {
                        self.collect(formal, actual, span);
                    }
                }
                Type::Array(Some(actual)) => {
                    let actuals = match &**actual {
                        Type::Union(members) => members.to_vec(),
                        other => vec![other.clone()],
                    };
                    for (formal, actual) in formals.iter().zip(actuals.iter()) {
                        self.collect(formal, actual, span);
                    }
                }
                _ => {}
            },
            Type::Object {
                id: formal_id,
                args: formal_args,
                ..
            } => {
                if let Type::Object {
                    id: actual_id,
                    args: actual_args,
                    ..
                } = actual
                    && formal_id == actual_id
                {
                    for (formal, actual) in formal_args.iter().zip(actual_args.iter()) {
                        self.collect(formal, actual, span);
                    }
                }
            }
            Type::Function(Some(formal_fn)) => {
                if let Type::Function(Some(actual_fn)) = actual {
                    for (formal, actual) in formal_fn.params.iter().zip(actual_fn.params.iter()) {
                        self.collect(formal, actual, span);
                    }
                    for (formal, actual) in formal_fn.returns.iter().zip(actual_fn.returns.iter()) {
                        self.collect(formal, actual, span);
                    }
                }
            }
            // a union contributes whichever member carries variables, `nil`
            // members and concrete members simply produce nothing
            Type::Union(formals) => {
                for formal in formals.clone() {
                    if self.mentions(&formal) {
                        self.collect(&formal, actual, span);
                    }
                }
            }
            Type::Rec(inner) => self.collect(inner, actual, span),
            _ => {}
        }
    }

    /// `T = candidate`, the first binding wins, a compatible later candidate
    /// widens it and an incompatible one makes the variable ambiguous
    fn bind(&mut self, var: &str, candidate: Type, span: Span) -> bool {
        if !Self::has_info(&candidate) {
            return false;
        }
        match self.solution.bindings.get(var) {
            Some(known) if *known == candidate => false,
            Some(known) => {
                if known.accepts(&candidate) || candidate.accepts(known) {
                    let widened = known.clone() | candidate;
                    self.solution.bindings.insert(var.into(), widened);
                    true
                } else {
                    if !self.is_ambiguous(var) {
                        self.solution.diagnostics.push(Diagnostic::Ambiguous {
                            name: var.into(),
                            first: known.clone(),
                            second: candidate,
                            span,
                        });
                    }
                    false
                }
            }
            None => {
                self.solution.bindings.insert(var.into(), candidate);
                true
            }
        }
    }

    fn is_ambiguous(&self, var: &str) -> bool {
        self.solution
            .diagnostics
            .iter()
            .any(|d| matches!(d, Diagnostic::Ambiguous { name, .. } if name.as_ref() == var))
    }

    fn solve_equalities(&mut self) -> bool {
        let mut changed = false;
        for eq in std::mem::take(&mut self.equalities) {
            let candidate = self.substitute(&eq.candidate);
            changed |= self.bind(&eq.var, candidate, eq.span);
        }
        changed
    }

    /// An unbound variable with a concrete bound takes the bound as its type
    fn solve_bounds_forward(&mut self) -> bool {
        let mut changed = false;
        for var in self.vars.clone() {
            if self.solution.bindings.contains_key(&var.name) {
                continue;
            }
            let Some(bound) = &var.bound else { continue };
            let bound = self.substitute(bound);
            if self.mentions(&bound) || !Self::has_info(&bound) {
                continue;
            }
            self.solution.bindings.insert(var.name.clone(), bound);
            changed = true;
        }
        changed
    }

    /// `T := array<int>` together with `T: array<V>` determines `V`
    fn solve_bounds_inverse(&mut self) -> bool {
        let mut changed = false;
        for var in self.vars.clone() {
            let Some(bound) = &var.bound else { continue };
            let bound = self.substitute(bound);
            let Some(known) = self.solution.bindings.get(&var.name).cloned() else {
                continue;
            };
            if !Self::has_info(&bound) || !self.mentions(&bound) {
                continue;
            }
            let mut inferred = vec![];
            self.match_template(&bound, &known, var.span, &mut inferred);
            for eq in inferred {
                changed |= self.bind(&eq.var, eq.candidate, eq.span);
            }
        }
        changed
    }

    /// Matches a concrete type against a template, binding the variables the
    /// template mentions
    fn match_template(
        &mut self,
        template: &Type,
        concrete: &Type,
        span: Span,
        out: &mut Vec<Equality>,
    ) {
        match template {
            Type::Param(name) if self.is_var(name) => out.push(Equality {
                var: name.clone(),
                candidate: concrete.clone(),
                span,
            }),
            Type::Array(template_inner) => {
                if let Type::Array(Some(concrete_inner)) = concrete
                    && let Some(template_inner) = template_inner
                {
                    self.match_template(template_inner, concrete_inner, span, out);
                }
            }
            Type::Table(template_k, template_v) => {
                if let Type::Table(concrete_k, concrete_v) = concrete {
                    if let (Some(tk), Some(ck)) = (template_k.as_deref(), concrete_k.as_deref()) {
                        self.match_template(tk, ck, span, out);
                    }
                    if let (Some(tv), Some(cv)) = (template_v.as_deref(), concrete_v.as_deref()) {
                        self.match_template(tv, cv, span, out);
                    }
                }
            }
            Type::Union(templates) => {
                for template in templates.clone() {
                    self.match_template(&template, concrete, span, out);
                }
            }
            Type::TypeTuple(templates) => {
                if let Type::TypeTuple(concretes) = concrete {
                    for (template, concrete) in templates.iter().zip(concretes.iter()) {
                        self.match_template(template, concrete, span, out);
                    }
                }
            }
            Type::TypeTable(templates) => {
                if let Type::TypeTable(concretes) = concrete {
                    for (key, template) in templates.iter() {
                        if let Some((_, concrete)) = concretes.iter().find(|(k, _)| k == key) {
                            self.match_template(template, concrete, span, out);
                        }
                    }
                }
            }
            Type::Object {
                id: template_id,
                args: template_args,
                ..
            } => {
                if let Type::Object {
                    id: concrete_id,
                    args: concrete_args,
                    ..
                } = concrete
                    && template_id == concrete_id
                {
                    for (template, concrete) in template_args.iter().zip(concrete_args.iter()) {
                        self.match_template(template, concrete, span, out);
                    }
                }
            }
            Type::Function(Some(template_fn)) => {
                if let Type::Function(Some(concrete_fn)) = concrete {
                    for (template, concrete) in
                        template_fn.params.iter().zip(concrete_fn.params.iter())
                    {
                        self.match_template(template, concrete, span, out);
                    }
                    for (template, concrete) in
                        template_fn.returns.iter().zip(concrete_fn.returns.iter())
                    {
                        self.match_template(template, concrete, span, out);
                    }
                }
            }
            Type::Rec(inner) => self.match_template(inner, concrete, span, out),
            _ => {}
        }
    }

    /// Every binding has to satisfy its declared bound
    fn check_bounds(&mut self) {
        for var in self.vars.clone() {
            let Some(binding) = self.solution.bindings.get(&var.name).cloned() else {
                continue;
            };
            let Some(bound) = &var.bound else { continue };
            let bound = self.substitute(bound);
            // a bound that still mentions a variable cannot be checked
            if !Self::has_info(&bound) || self.mentions(&bound) || bound.accepts(&binding) {
                continue;
            }
            self.solution.diagnostics.push(Diagnostic::BoundViolated {
                name: var.name.clone(),
                bound,
                candidate: binding,
                span: var.span,
            });
        }
    }
}

impl Solution {
    /// Replaces every variable this solution decided, inside a type.
    ///
    /// A variable the solution says nothing about is left as it is rather than
    /// being answered with `any`: an undecided variable is still a question, and
    /// answering it would make every check that reads the result pass.
    pub fn substitute(&self, ty: &Type) -> Type {
        match ty {
            Type::Param(name) => self
                .bindings
                .get(name)
                .cloned()
                .unwrap_or_else(|| ty.clone()),
            Type::Array(Some(inner)) => Type::Array(Some(Box::new(self.substitute(inner)))),
            Type::Array(None) => Type::Array(None),
            Type::Table(k, v) => Type::Table(
                k.as_deref().map(|k| Box::new(self.substitute(k))),
                v.as_deref().map(|v| Box::new(self.substitute(v))),
            ),
            Type::Union(ts) => flatten(ts.iter().map(|t| self.substitute(t)).collect()),
            Type::TypeTuple(ts) => Type::TypeTuple(ts.iter().map(|t| self.substitute(t)).collect()),
            Type::TypeTable(fields) => Type::TypeTable(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Box::new(self.substitute(v))))
                    .collect(),
            ),
            Type::Object {
                id,
                name,
                base,
                args,
            } => Type::Object {
                id: *id,
                name: name.clone(),
                base: *base,
                args: args.iter().map(|t| self.substitute(t)).collect(),
            },
            Type::Function(Some(ft)) => Type::Function(Some(FunctionType {
                params: ft.params.iter().map(|t| self.substitute(t)).collect(),
                var_arg: ft.var_arg,
                returns: ft.returns.iter().map(|t| self.substitute(t)).collect(),
                return_var_arg: ft.return_var_arg,
            })),
            Type::Rec(inner) => Type::Rec(Box::new(self.substitute(inner))),
            other => other.clone(),
        }
    }
}

/// `int | nil` substituted into `T | nil` must not produce a nested union,
/// `accepts` compares unions member by member
fn flatten(types: Vec<Type>) -> Type {
    let mut flat: Vec<Type> = vec![];
    for ty in types {
        match ty {
            Type::Union(members) => flat.extend(members.into_vec()),
            other => flat.push(other),
        }
    }
    flat.dedup();
    match flat.len() {
        1 => flat.pop().unwrap_or(Type::Any),
        _ => Type::Union(flat.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duka_shared::errors::Position;
    use duka_shared::value::ConstValue;

    fn span() -> Span {
        Span {
            start: Position {
                line: 1,
                column: 1,
                at_char: 0,
            },
            end: Position {
                line: 1,
                column: 2,
                at_char: 1,
            },
        }
    }

    fn var(name: &str) -> VarDecl {
        VarDecl {
            name: name.into(),
            bound: None,
            default: None,
            span: span(),
            shape: ParamShape::Fixed,
        }
    }

    fn with_bound(name: &str, bound: Type) -> VarDecl {
        VarDecl {
            bound: Some(bound),
            ..var(name)
        }
    }

    fn with_default(name: &str, default: Type) -> VarDecl {
        VarDecl {
            default: Some(default),
            ..var(name)
        }
    }

    fn param(name: &str) -> Type {
        Type::Param(name.into())
    }

    fn array(inner: Type) -> Type {
        Type::Array(Some(Box::new(inner)))
    }

    fn record(field: &str, ty: Type) -> Type {
        Type::TypeTable(
            [(
                ConstValue::String(field.as_bytes().to_vec().into_boxed_slice()),
                Box::new(ty),
            )]
            .into(),
        )
    }

    fn fn_type(param_ty: Type, return_ty: Type) -> Type {
        Type::Function(Some(FunctionType {
            params: [param_ty].into(),
            var_arg: false,
            returns: [return_ty].into(),
            return_var_arg: false,
        }))
    }

    /// pairs a single parameter against a single argument
    fn pairs(formal: &Type, actual: &Type) -> Vec<(Box<str>, Type)> {
        let mut solver = Solver::new(vec![var("T"), var("V")]);
        solver.constrain_call(
            std::slice::from_ref(formal),
            std::slice::from_ref(actual),
            span(),
        );
        solver
            .equalities
            .iter()
            .map(|e| (e.var.clone(), e.candidate.clone()))
            .collect()
    }

    /// binds a variable from one argument and solves
    fn solve_from(vars: Vec<VarDecl>, var: &str, candidate: Type) -> Solution {
        let mut solver = Solver::new(vars);
        solver.constrain_call(&[param(var)], &[candidate], span());
        solver.solve(vec![], span()).clone()
    }

    #[test]
    fn array_binds_element_not_whole() {
        assert_eq!(
            pairs(&array(param("T")), &array(Type::Int)),
            vec![("T".into(), Type::Int)]
        );
    }

    #[test]
    fn tuple_actual_binds_every_element() {
        // a mixed tuple constrains `T` twice, which the solver reports later
        assert_eq!(
            pairs(
                &array(param("T")),
                &Type::TypeTuple(vec![Type::Int, Type::String])
            ),
            vec![("T".into(), Type::Int), ("T".into(), Type::String)]
        );
    }

    #[test]
    fn record_binds_field_value() {
        assert_eq!(
            pairs(&record("value", param("T")), &record("value", Type::Int)),
            vec![("T".into(), Type::Int)]
        );
    }

    #[test]
    fn nested_record_binds_inner_param() {
        assert_eq!(
            pairs(
                &record("value", array(param("T"))),
                &record("value", array(Type::Int))
            ),
            vec![("T".into(), Type::Int)]
        );
    }

    #[test]
    fn table_binds_key_and_value_apart() {
        let formal = Type::Table(Some(Box::new(param("T"))), Some(Box::new(param("V"))));
        let actual = Type::Table(Some(Box::new(Type::String)), Some(Box::new(Type::Int)));
        let mut pairs = pairs(&formal, &actual);
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            pairs,
            vec![("T".into(), Type::String), ("V".into(), Type::Int)]
        );
    }

    #[test]
    fn function_param_binds_returns_too() {
        assert_eq!(
            pairs(
                &fn_type(param("T"), param("T")),
                &fn_type(Type::Int, Type::Int)
            ),
            vec![("T".into(), Type::Int), ("T".into(), Type::Int)]
        );
    }

    #[test]
    fn object_args_pair_on_the_same_id() {
        let formal = Type::Object {
            id: 1,
            name: "Box".into(),
            base: None,
            args: [param("T").into()].into(),
        };
        let actual = Type::Object {
            id: 1,
            name: "Box".into(),
            base: None,
            args: [Type::Int.into()].into(),
        };
        assert_eq!(pairs(&formal, &actual), vec![("T".into(), Type::Int)]);
    }

    #[test]
    fn nothing_to_infer_from() {
        for actual in [
            Type::Nil,
            Type::Any,
            Type::Table(None, None),
            Type::Array(None),
        ] {
            assert!(pairs(&param("T"), &actual).is_empty(), "{actual:?}");
        }
    }

    #[test]
    fn foreign_param_is_ignored() {
        // recursion markers must never be bound
        assert!(pairs(&param("__rec_Foo"), &Type::Int).is_empty());
    }

    #[test]
    fn shape_mismatch_binds_nothing() {
        assert!(pairs(&array(param("T")), &Type::Int).is_empty());
        assert!(pairs(&record("value", param("T")), &Type::Int).is_empty());
    }

    #[test]
    fn infers_from_argument() {
        let solution = solve_from(vec![var("T")], "T", Type::Int);
        assert_eq!(solution.bindings.get("T"), Some(&Type::Int));
        assert!(solution.diagnostics.is_empty());
    }

    #[test]
    fn conflicting_candidates_are_ambiguous() {
        let mut solver = Solver::new(vec![var("T")]);
        solver.constrain_call(
            &[param("T"), param("T")],
            &[Type::Int, Type::String],
            span(),
        );
        assert!(matches!(
            solver.solve(vec![], span()).diagnostics.as_slice(),
            [Diagnostic::Ambiguous { .. }]
        ));
    }

    #[test]
    fn compatible_candidates_widen() {
        // an int argument satisfies a float variable, this is not a conflict
        let mut solver = Solver::new(vec![var("T")]);
        solver.constrain_call(&[param("T"), param("T")], &[Type::Int, Type::Float], span());
        let solution = solver.solve(vec![], span());
        assert!(
            solution.diagnostics.is_empty(),
            "{:?}",
            solution.diagnostics
        );
        assert!(solution.bindings["T"].accepts(&Type::Int));
        assert!(solution.bindings["T"].accepts(&Type::Float));
    }

    #[test]
    fn nil_candidate_does_not_conflict() {
        let mut solver = Solver::new(vec![var("T")]);
        solver.constrain_call(&[param("T"), param("T")], &[Type::Int, Type::Nil], span());
        let solution = solver.solve(vec![], span());
        assert_eq!(solution.bindings.get("T"), Some(&Type::Int));
        assert!(solution.diagnostics.is_empty());
    }

    #[test]
    fn falls_back_to_bound() {
        let mut solver = Solver::new(vec![with_bound("T", Type::Int)]);
        assert_eq!(solver.solve(vec![], span()).bindings["T"], Type::Int);
    }

    #[test]
    fn falls_back_to_default_then_any() {
        let mut solver = Solver::new(vec![with_default("T", Type::String)]);
        assert_eq!(solver.solve(vec![], span()).bindings["T"], Type::String);
        let mut solver = Solver::new(vec![var("T")]);
        assert_eq!(solver.solve(vec![], span()).bindings["T"], Type::Any);
    }

    #[test]
    fn bound_is_checked_after_inference() {
        let solution = solve_from(vec![with_bound("T", Type::Int)], "T", Type::String);
        assert!(matches!(
            solution.diagnostics.as_slice(),
            [Diagnostic::BoundViolated { .. }]
        ));
    }

    #[test]
    fn param_bound_uses_the_other_variable() {
        let mut solver = Solver::new(vec![with_bound("T", param("V")), var("V")]);
        solver.constrain_call(&[param("T"), param("V")], &[Type::Int, Type::Int], span());
        assert!(solver.solve(vec![], span()).diagnostics.is_empty());
    }

    #[test]
    fn param_bound_violation_is_reported() {
        let mut solver = Solver::new(vec![with_bound("T", param("V")), var("V")]);
        solver.constrain_call(
            &[param("T"), param("V")],
            &[Type::Int, Type::String],
            span(),
        );
        assert!(matches!(
            solver.solve(vec![], span()).diagnostics.as_slice(),
            [Diagnostic::BoundViolated { .. }]
        ));
    }

    #[test]
    fn bound_template_is_matched_backwards() {
        let mut solver = Solver::new(vec![with_bound("T", array(param("V"))), var("V")]);
        solver.constrain_call(&[param("T")], &[array(Type::Int)], span());
        let solution = solver.solve(vec![], span());
        assert_eq!(solution.bindings.get("V"), Some(&Type::Int));
        assert!(
            solution.diagnostics.is_empty(),
            "{:?}",
            solution.diagnostics
        );
    }

    #[test]
    fn explicit_type_arguments_win() {
        let mut solver = Solver::new(vec![var("T")]);
        solver.constrain_call(&[param("T")], &[Type::Int], span());
        let solution = solver.solve(vec![Type::String], span());
        assert_eq!(solution.bindings.get("T"), Some(&Type::String));
        assert!(solution.diagnostics.is_empty());
    }

    #[test]
    fn explicit_type_arguments_arity_is_checked() {
        let mut solver = Solver::new(vec![var("T")]);
        assert!(matches!(
            solver
                .solve(vec![Type::Int, Type::String], span())
                .diagnostics
                .as_slice(),
            [Diagnostic::ArityMismatch {
                expected: 1,
                given: 2,
                ..
            }]
        ));
    }

    #[test]
    fn substitute_reaches_nested_positions() {
        let mut solver = Solver::new(vec![var("T")]);
        solver.constrain_call(&[param("T")], &[Type::Int], span());
        solver.solve(vec![], span());
        let ty = array(record("value", param("T")));
        assert_eq!(solver.substitute(&ty), array(record("value", Type::Int)));
    }

    #[test]
    fn substitute_keeps_unknown_params() {
        let solver = Solver::new(vec![var("T")]);
        assert_eq!(solver.substitute(&param("Z")), param("Z"));
    }
}
