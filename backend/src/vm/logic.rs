use std::collections::HashMap;

use duka_shared::utils::UniqueVec;

use crate::codegen::logic::{CompiledQuery, LogicProto};
use crate::instructions::logic::LogicInstruction as I;

#[derive(Debug, Clone)]
enum WVal {
    Unbound,
    Str(usize),
    Ref(usize),
}

fn deref(regs: &[WVal], mut s: usize) -> usize {
    while let WVal::Ref(t) = &regs[s] {
        s = *t;
    }
    s
}

fn unify(regs: &mut [WVal], trail: &mut Vec<(usize, WVal)>, a: usize, b: usize) -> bool {
    let da = deref(regs, a);
    let db = deref(regs, b);
    if da == db {
        return true;
    }
    match (&regs[da], &regs[db]) {
        (WVal::Unbound, _) => {
            trail.push((da, WVal::Unbound));
            regs[da] = WVal::Ref(db);
            true
        }
        (_, WVal::Unbound) => {
            trail.push((db, WVal::Unbound));
            regs[db] = WVal::Ref(da);
            true
        }
        (WVal::Str(ca), WVal::Str(cb)) => ca == cb,
        _ => false,
    }
}

fn bind_const(regs: &mut [WVal], trail: &mut Vec<(usize, WVal)>, s: usize, c: usize) -> bool {
    let d = deref(regs, s);
    match &regs[d] {
        WVal::Unbound => {
            trail.push((d, WVal::Unbound));
            regs[d] = WVal::Str(c);
            true
        }
        WVal::Str(v) => *v == c,
        _ => false,
    }
}

struct Choice {
    pc: usize,
    trail_len: usize, // push进choices时候的trail的长度, 用于归零防止影响下一个分支
}

pub struct Wam {
    regs: Vec<WVal>,           //寄存器
    trail: Vec<(usize, WVal)>, //用于恢复, 记录差异
    choices: Vec<Choice>,
    strings: UniqueVec<String>, //字符串池
    pc: usize,
    code: Vec<I>,
    call_stack: Vec<usize>,                 //调用栈
    solutions: Vec<HashMap<usize, String>>, //记录解
    collect: Vec<usize>,
}

impl Wam {
    pub fn new(proto: &LogicProto, query: &CompiledQuery, collect_regs: Vec<usize>) -> Self {
        let mut code = query.instructions.clone();
        let mut reg_count = 8;
        for proc in &proto.procedures {
            reg_count = reg_count.max(proc.arity + 4);
        }

        // resolve Call targets: find the starting instruction index for each procedure
        let proc_starts: Vec<usize> = proto
            .procedures
            .iter()
            .map(|proc| {
                let start = code.len();
                if proc.clauses.len() > 1 {
                    let mut sizes = Vec::new();
                    for (i, clause) in proc.clauses.iter().enumerate() {
                        if i < proc.clauses.len() - 1 {
                            sizes.push(1 + clause.len());
                        } else {
                            sizes.push(clause.len());
                        }
                    }
                    let mut cum_offsets = Vec::new();
                    let mut cum = 0usize;
                    for s in &sizes {
                        cum_offsets.push(cum);
                        cum += s;
                    }
                    for (i, clause) in proc.clauses.iter().enumerate() {
                        if i < proc.clauses.len() - 1 {
                            let target = (start + cum_offsets[i + 1]) as u8;
                            code.push(I::TRY(target, 0));
                        }
                        code.extend_from_slice(clause);
                    }
                } else if proc.clauses.len() == 1 {
                    code.extend_from_slice(&proc.clauses[0]);
                }
                start
            })
            .collect();

        // patch Call instructions to jump to procedure starts
        let mut raw_code: Vec<u32> = code.iter().map(|i| i.raw()).collect();
        for i in 0..raw_code.len() {
            let inst = I::from_raw(raw_code[i]);
            if let Ok(decoded) = inst.decode() {
                use crate::instructions::logic::DecodeLogicInstruction::*;
                let new_raw = match decoded {
                    Call(_) => {
                        let Call(idx) = decoded else { continue };
                        let addr = proc_starts.get(idx as usize).copied().unwrap_or(0);
                        I::Call(addr as u8).raw()
                    }
                    _ => inst.raw(),
                };
                raw_code[i] = new_raw;
            }
        }
        let code: Vec<I> = raw_code.into_iter().map(I::from_raw).collect();

        Wam {
            regs: vec![WVal::Unbound; reg_count],
            trail: vec![],
            choices: vec![],
            strings: proto.strings.clone(),
            pc: 0,
            code,
            call_stack: vec![],
            solutions: vec![],
            collect: collect_regs,
        }
    }

    fn backtrack(&mut self) -> bool {
        if let Some(ch) = self.choices.pop() {
            //self.regs.clone_from_slice(&ch.saved); NO
            for (reg, old) in self.trail.drain(ch.trail_len..).rev() {
                self.regs[reg] = old;
            }
            self.pc = ch.pc;
            return true;
        }
        false
    }

    pub fn run(&mut self) -> Vec<HashMap<usize, String>> {
        self.solutions.clear();
        self.pc = 0;

        loop {
            if self.pc >= self.code.len() {
                break;
            }
            let decoded = match self.code[self.pc].decode() {
                Ok(d) => d,
                Err(_) => break,
            };

            use crate::instructions::logic::DecodeLogicInstruction as D;
            match decoded {
                D::UnifyConst(n, c) => {
                    if !bind_const(&mut self.regs, &mut self.trail, n as usize, c as usize) {
                        if !self.backtrack() {
                            break;
                        }
                        continue;
                    }
                    self.pc += 1;
                }
                D::UnifyVar(_n) => {
                    self.pc += 1;
                }
                D::UnifyVarVar(a, b) => {
                    if !unify(&mut self.regs, &mut self.trail, a as usize, b as usize) {
                        if !self.backtrack() {
                            break;
                        }
                        continue;
                    }
                    self.pc += 1;
                }
                D::UnifyVarConst(a, b) => {
                    if !unify(&mut self.regs, &mut self.trail, a as usize, b as usize) {
                        if !self.backtrack() {
                            break;
                        }
                        continue;
                    }
                    self.pc += 1;
                }
                D::BindVar(a, b) => {
                    let db = deref(&self.regs, b as usize);
                    match self.regs[db] {
                        WVal::Str(_) => {
                            if !unify(&mut self.regs, &mut self.trail, a as usize, b as usize) {
                                if !self.backtrack() {
                                    break;
                                }
                                continue;
                            }
                        }
                        _ => {
                            let da = deref(&self.regs, a as usize);
                            if da != db {
                                self.trail.push((da, WVal::Unbound));
                                self.regs[da] = WVal::Ref(db);
                            }
                        }
                    }
                    self.pc += 1;
                }
                D::BindConst(a, b) => {
                    let db = deref(&self.regs, b as usize);
                    let c = match self.regs[db] {
                        WVal::Str(c) => c,
                        _ => {
                            self.pc += 1;
                            continue;
                        }
                    };
                    if !bind_const(&mut self.regs, &mut self.trail, a as usize, c) {
                        if !self.backtrack() {
                            break;
                        }
                        continue;
                    }
                    self.pc += 1;
                }
                D::Succeed() => {
                    let mut sol = HashMap::new();
                    for &r in &self.collect {
                        let d = deref(&self.regs, r);
                        if let WVal::Str(c) = &self.regs[d] {
                            sol.insert(r, self.strings.get(*c).expect("NO").clone());
                        }
                    }
                    self.solutions.push(sol);
                    if !self.backtrack() {
                        break;
                    }
                }
                D::Fail() => {
                    if !self.backtrack() {
                        break;
                    }
                }
                D::Call(target) => {
                    self.call_stack.push(self.pc + 1);
                    self.pc = target as usize;
                }
                D::Proceed() => {
                    if let Some(ret) = self.call_stack.pop() {
                        self.pc = ret;
                    } else {
                        break;
                    }
                }
                D::TRY(addr, _) => {
                    self.choices.push(Choice {
                        pc: addr as usize,
                        // saved: self.regs.clone(), NO
                        trail_len: self.trail.len(),
                    });
                    self.pc += 1;
                }
                _ => self.pc += 1,
            }
        }

        std::mem::take(&mut self.solutions)
    }
}

pub fn execute_query(
    proto: &LogicProto,
    query_idx: usize,
) -> Result<Vec<HashMap<usize, String>>, String> {
    let query = proto
        .queries
        .get(query_idx)
        .ok_or_else(|| format!("query index {query_idx} out of bounds"))?;

    // collect all registers that appear as Var positions in the query
    let mut regs = vec![];
    for inst in &query.instructions {
        if let Ok(d) = inst.decode() {
            use crate::instructions::logic::DecodeLogicInstruction::*;
            match d {
                UnifyVar(n) => {
                    if !regs.contains(&(n as usize)) {
                        regs.push(n as usize);
                    }
                }
                UnifyConst(n, _) => {
                    if !regs.contains(&(n as usize)) {
                        regs.push(n as usize);
                    }
                }
                _ => {}
            }
        }
    }
    regs.sort();

    let mut wam = Wam::new(proto, query, regs);
    Ok(wam.run())
}
