use std::collections::BTreeMap;

use duka_shared::{
    errors::{DukaIRError, DukaIRErrorKind},
    types::{DukaGenerator, Fact, Goal, LogicDatabase, Query, QueryCount, Rule, Term},
    utils::UniqueVec,
};

use crate::instructions::logic::LogicInstruction as I;

#[derive(Debug, Clone, PartialEq)]
pub struct LogicProto {
    pub procedures: Vec<Procedure>,
    pub queries: Vec<CompiledQuery>,
    pub strings: UniqueVec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Procedure {
    pub name: String,
    pub arity: usize,
    pub clauses: Vec<Vec<I>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledQuery {
    pub instructions: Vec<I>,
    pub count: QueryCount,
}

fn compile_term(code: &mut Vec<I>, strings: &mut UniqueVec<String>, slot: u8, term: &Term) {
    match term {
        Term::Atom(s) | Term::String(s) => {
            code.push(I::UnifyConst(slot, strings.push(s.to_string()) as u8));
        }
        Term::Number(n) => {
            code.push(I::UnifyConst(slot, strings.push(n.to_string()) as u8));
        }
        Term::Bool(b) => code.push(I::UnifyConst(
            slot,
            strings.push((if *b { "true" } else { "false" }).to_owned()) as u8,
        )),
        Term::Anonymous => {}
        Term::Var(_) => code.push(I::UnifyVar(slot)),
        _ => {}
    }
}

fn compile_call(
    code: &mut Vec<I>,
    strings: &mut UniqueVec<String>,
    name: &str,
    args: &[Term],
    procs: &[Procedure],
) -> Result<(), DukaIRError> {
    let idx = procs
        .iter()
        .position(|p| p.name == name && p.arity == args.len())
        .ok_or_else(|| {
            DukaIRError::from(DukaIRErrorKind::Custom(format!(
                "unknown predicate `{name}`"
            )))
        })?;
    for (i, arg) in args.iter().enumerate() {
        match arg {
            Term::Atom(s) | Term::String(s) => {
                code.push(I::UnifyConst(i as u8, strings.push(s.to_string()) as u8));
            }
            Term::Number(n) => {
                code.push(I::UnifyConst(i as u8, strings.push(n.to_string()) as u8));
            }
            Term::Bool(b) => code.push(I::UnifyConst(
                i as u8,
                strings.push((if *b { "true" } else { "false" }).to_owned()) as u8,
            )),
            Term::Anonymous => {}
            Term::Var(_) => code.push(I::UnifyVar(i as u8)),
            _ => {}
        }
    }
    code.push(I::Call(idx as u8));
    Ok(())
}

fn compile_goal(
    code: &mut Vec<I>,
    strings: &mut UniqueVec<String>,
    procs: &[Procedure],
    goal: &Goal,
) -> Result<(), DukaIRError> {
    match goal {
        Goal::Term(Term::Compound(name, args)) => compile_call(code, strings, name, args, procs),
        Goal::Meta(name, args) => compile_call(code, strings, name, args, procs),
        Goal::And(goals) => {
            for g in goals {
                compile_goal(code, strings, procs, g)?;
            }
            Ok(())
        }
        _ => Err(DukaIRError::from("unsupported goal type")),
    }
}

#[derive(Default)]
pub struct LogicGenerator {
    strings: UniqueVec<String>,
    clauses: Vec<(String, usize, Vec<I>, Option<Goal>)>,
    queries: Vec<Query>,
}

impl LogicGenerator {
    fn add_fact(&mut self, Fact(name, terms): Fact) {
        let arity = terms.len();
        let mut code = vec![];
        for (i, t) in terms.iter().enumerate() {
            compile_term(&mut code, &mut self.strings, i as u8, t);
        }
        code.push(I::Succeed());
        self.clauses.push((name, arity, code, None));
    }

    fn add_rule(&mut self, Rule(name, terms, goal): Rule) {
        let arity = terms.len();
        let mut code = vec![];
        for (i, t) in terms.iter().enumerate() {
            compile_term(&mut code, &mut self.strings, i as u8, t);
        }
        self.clauses.push((name, arity, code, Some(goal)));
    }

    fn add_query(&mut self, query: Query) {
        self.queries.push(query);
    }

    fn build(mut self) -> Result<LogicProto, DukaIRError> {
        let mut groups: BTreeMap<(String, usize), Vec<(Vec<I>, Option<Goal>)>> = BTreeMap::new();
        for (name, arity, head, body) in self.clauses.drain(..) {
            groups
                .entry((name.clone(), arity))
                .or_default()
                .push((head, body));
        }

        let names: Vec<(String, usize)> = groups.keys().cloned().collect();
        let mut raw: Vec<Vec<(Vec<I>, Option<Goal>)>> =
            names.iter().map(|k| groups.remove(k).unwrap()).collect();

        let temp: Vec<Procedure> = names
            .iter()
            .zip(raw.iter())
            .map(|(k, entries)| {
                let clauses = entries.iter().map(|(h, _)| h.clone()).collect();
                Procedure {
                    name: k.0.clone(),
                    arity: k.1,
                    clauses,
                }
            })
            .collect();

        for entries in raw.iter_mut() {
            for (head, body) in entries.iter_mut() {
                if let Some(goal) = body.take() {
                    let mut bc = vec![];
                    compile_goal(&mut bc, &mut self.strings, &temp, &goal)?;
                    bc.push(I::Proceed());
                    head.extend(bc);
                }
            }
        }

        let procedures: Vec<Procedure> = names
            .into_iter()
            .zip(raw)
            .map(|(k, entries)| {
                let clauses = entries.into_iter().map(|(h, _)| h).collect();
                Procedure {
                    name: k.0,
                    arity: k.1,
                    clauses,
                }
            })
            .collect();

        let queries: Result<Vec<CompiledQuery>, _> = self
            .queries
            .drain(..)
            .map(|q| {
                let mut code = vec![];
                compile_goal(&mut code, &mut self.strings, &procedures, &q.0)?;
                Ok::<_, DukaIRError>(CompiledQuery {
                    instructions: code,
                    count: QueryCount::All,
                })
            })
            .collect();

        Ok(LogicProto {
            procedures,
            queries: queries?,
            strings: self.strings,
        })
    }
}

impl DukaGenerator<LogicProto> for LogicGenerator {
    type InputType = LogicDatabase;
    type ConfigType = ();
    fn generate(chunk: Self::InputType, _: Self::ConfigType) -> Result<LogicProto, DukaIRError> {
        let mut g = LogicGenerator::default();
        for fact in chunk.facts {
            g.add_fact(fact);
        }
        for rule in chunk.rules {
            g.add_rule(rule);
        }
        for query in chunk.queries.into_vec() {
            g.add_query(query);
        }
        g.build()
    }
}
