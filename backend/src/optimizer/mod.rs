//! Backend-specific optimizations over a compiled prototype

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use duka_shared::{dtype::Type, errors::Span, types::DukaOptimizer, value::ConstValue};

use crate::instructions::{DecodeInstruction, Instruction};
use crate::value::DukaProto;

#[derive(Debug, Clone, Default)]
pub struct TypeEvidence {
    pub types: Arc<HashMap<Span, Arc<Type>>>,
}

impl TypeEvidence {
    #[inline]
    pub fn new(types: HashMap<Span, Arc<Type>>) -> Self {
        Self {
            types: Arc::new(types),
        }
    }

    #[inline]
    pub fn at(&self, span: Span) -> Option<&Type> {
        self.types.get(&span).map(|ty| &**ty)
    }
}

/// What a pass is allowed to lean on besides the prototype itself.
#[derive(Debug, Clone, Default)]
pub struct OptimizeConfig {
    pub evidence: Option<TypeEvidence>,
}

/// Local rewrites over one prototype's instruction stream.
#[derive(Debug, Clone, Copy, Default)]
pub struct PeepholeOptimizer;

impl DukaOptimizer<DukaProto> for PeepholeOptimizer {
    type ConfigType = OptimizeConfig;

    fn optimize(proto: &mut DukaProto, config: OptimizeConfig) {
        optimize_proto(proto, config.evidence.as_ref());
    }
}

/// Runs the optimizer over a prototype and everything nested in it.
#[inline(always)]
pub fn optimize(proto: &mut DukaProto, config: OptimizeConfig) {
    PeepholeOptimizer::optimize(proto, config)
}

fn optimize_proto(proto: &mut DukaProto, evidence: Option<&TypeEvidence>) {
    for nested in proto.nested_protos.iter_mut() {
        optimize_proto(nested, evidence);
    }
    peephole(proto, evidence);
}

/// The target a branch instruction means, in absolute instruction positions.
fn branch_target(pc: usize, decoded: &DecodeInstruction) -> Option<isize> {
    let pc = pc as isize;
    Some(match decoded {
        DecodeInstruction::Jump(off) => pc + *off as isize,
        // the not-taken arm of a fused test
        DecodeInstruction::TestJump(_, _, off) => pc + *off as isize,
        DecodeInstruction::ForPrepare(_, k) | DecodeInstruction::TForPrepare(_, k) => {
            pc + *k as isize
        }
        DecodeInstruction::ForLoop(_, k) | DecodeInstruction::TForLoop(_, k) => pc - *k as isize,
        _ => return None,
    })
}

/// The next position at or after `from` that survived, or the end.
fn map_at_or_after(map: &[usize], from: usize) -> usize {
    for pos in from..map.len() {
        if map[pos] != usize::MAX {
            return map[pos];
        }
    }
    map.last().copied().unwrap_or(0)
}

fn span_of(proto: &DukaProto, pc: usize) -> Option<Span> {
    proto
        .debug_info
        .inst_spans
        .iter()
        .find(|(range, _)| range.contains(&pc))
        .map(|(_, span)| *span)
}

fn peephole(proto: &mut DukaProto, evidence: Option<&TypeEvidence>) {
    let old: Vec<Instruction> = proto.instructions.to_vec();
    let n = old.len();
    if n < 2 {
        return;
    }

    // Decode once. An instruction this build cannot read means the stream is
    // not ours to rewrite.
    let mut decoded: Vec<DecodeInstruction> = Vec::with_capacity(n);
    for inst in &old {
        match inst.decode() {
            Ok(d) => decoded.push(d),
            Err(_) => return,
        }
    }

    // Where every branch lands. A branch into a removed instruction would
    // jump somewhere it was not written to, so those are never removed.
    let mut targeted: HashSet<usize> = HashSet::new();
    for (pc, d) in decoded.iter().enumerate() {
        let Some(target) = branch_target(pc, d) else {
            continue;
        };
        if target < 0 || target as usize > n {
            return;
        }
        targeted.insert(target as usize);
    }

    // A `Test` skips exactly the instruction after it, so that one is not free
    // to remove or fuse: dropping it would move the skip onto something else.
    let mut skipped: HashSet<usize> = HashSet::new();
    for pc in 0..n {
        if pc + 1 < n && matches!(decoded[pc], DecodeInstruction::Test(..)) {
            skipped.insert(pc + 1);
        }
    }

    let mut keep = vec![true; n];

    // `if true` / `if false`: the evidence on the test's span says the
    // condition is a literal boolean, so the branch is decided here. Ran
    // before fusion so the two passes cannot both claim the `Jump`.
    if let Some(evidence) = evidence {
        for pc in 0..n {
            if targeted.contains(&pc) || skipped.contains(&pc) {
                continue;
            }
            let DecodeInstruction::Test(_, want) = decoded[pc] else {
                continue;
            };
            // Only the `Test`+`Jump` shape: a `Test` guarding something else
            // (`and`/`or` lowering) is not a branch we may drop.
            if pc + 1 >= n || !matches!(decoded[pc + 1], DecodeInstruction::Jump(_)) {
                continue;
            }
            let Some(span) = span_of(proto, pc) else {
                continue;
            };
            let Some(Type::Literal(ConstValue::Bool(literal))) = evidence.at(span) else {
                continue;
            };
            if *literal == want {
                // never branches: the test and the jump it guards both go.
                // The jump may still be someone else's target, in which case
                // folding is not ours to make.
                if targeted.contains(&(pc + 1)) {
                    continue;
                }
                keep[pc] = false;
                keep[pc + 1] = false;
            } else {
                // always branches: dropping the test leaves the jump to run
                keep[pc] = false;
            }
        }
    }

    // A fused test: the register, the truth it wants, and the old target.
    let mut fused: Vec<Option<(u8, bool, usize)>> = vec![None; n];

    for pc in 0..n {
        if !keep[pc] {
            continue;
        }
        match &decoded[pc] {
            // writing a register into itself
            DecodeInstruction::Move(a, b)
                if a == b && !targeted.contains(&pc) && !skipped.contains(&pc) =>
            {
                keep[pc] = false;
            }
            // a jump to the very next instruction
            DecodeInstruction::Jump(off)
                if *off == 1 && !targeted.contains(&pc) && !skipped.contains(&pc) =>
            {
                keep[pc] = false;
            }
            // `Test` + `Jump`, the shape every condition compiles to: one
            // dispatch instead of two
            DecodeInstruction::Test(r, t)
                if pc + 1 < n
                    && !targeted.contains(&(pc + 1))
                    // the test itself must not be skipped by the one before it
                    && !skipped.contains(&pc) =>
            {
                if let DecodeInstruction::Jump(off) = decoded[pc + 1] {
                    let target = pc as isize + 1 + off as isize;
                    if target >= 0 && target as usize <= n {
                        fused[pc] = Some((*r, *t, target as usize));
                        keep[pc + 1] = false;
                    }
                }
            }
            _ => {}
        }
    }

    // Old position -> new position, with `n` kept as the one-past-the-end
    // sentinel a branch may land on.
    let mut map = vec![usize::MAX; n + 1];
    let mut new_len = 0usize;
    for (pc, kept) in keep.iter().enumerate() {
        if *kept {
            map[pc] = new_len;
            new_len += 1;
        }
    }
    map[n] = new_len;
    // Every branch has to land on an instruction that survived.
    for target in &targeted {
        if map[*target] == usize::MAX {
            return;
        }
    }
    if new_len == n {
        return;
    }

    let mut out: Vec<Instruction> = Vec::with_capacity(new_len);
    for pc in 0..n {
        if !keep[pc] {
            continue;
        }
        let new_pc = map[pc];
        if let Some((r, t, target)) = fused[pc] {
            let off = map[target] as isize - new_pc as isize;
            if !(i16::MIN as isize..=i16::MAX as isize).contains(&off) {
                return;
            }
            out.push(Instruction::TestJump(r, t, off as i16));
            continue;
        }
        let inst = match &decoded[pc] {
            DecodeInstruction::Jump(_) => {
                let target = branch_target(pc, &decoded[pc]).unwrap() as usize;
                let off = map[target] as isize - new_pc as isize;
                if !(-(1 << 24)..(1 << 24)).contains(&off) {
                    return;
                }
                Instruction::Jump(off as i32)
            }
            DecodeInstruction::TestJump(r, t, _) => {
                let target = branch_target(pc, &decoded[pc]).unwrap() as usize;
                let off = map[target] as isize - new_pc as isize;
                if !(i16::MIN as isize..=i16::MAX as isize).contains(&off) {
                    return;
                }
                Instruction::TestJump(*r, *t, off as i16)
            }
            DecodeInstruction::ForPrepare(a, _) => {
                let target = branch_target(pc, &decoded[pc]).unwrap() as usize;
                let off = map[target] as isize - new_pc as isize;
                if !(0..(1 << 17)).contains(&off) {
                    return;
                }
                Instruction::ForPrepare(*a, off as u32)
            }
            DecodeInstruction::TForPrepare(a, _) => {
                let target = branch_target(pc, &decoded[pc]).unwrap() as usize;
                let off = map[target] as isize - new_pc as isize;
                if !(0..(1 << 17)).contains(&off) {
                    return;
                }
                Instruction::TForPrepare(*a, off as u32)
            }
            DecodeInstruction::ForLoop(a, _) => {
                let target = branch_target(pc, &decoded[pc]).unwrap() as usize;
                let off = new_pc as isize - map[target] as isize;
                if !(0..(1 << 17)).contains(&off) {
                    return;
                }
                Instruction::ForLoop(*a, off as u32)
            }
            DecodeInstruction::TForLoop(a, _) => {
                let target = branch_target(pc, &decoded[pc]).unwrap() as usize;
                let off = new_pc as isize - map[target] as isize;
                if !(0..(1 << 17)).contains(&off) {
                    return;
                }
                Instruction::TForLoop(*a, off as u32)
            }
            _ => old[pc].clone(),
        };
        out.push(inst);
    }

    // Keep the spans pointing at the instructions they described.
    let mut spans: Vec<(std::ops::Range<usize>, Span)> = Vec::new();
    for (range, span) in proto.debug_info.inst_spans.iter() {
        let start = map_at_or_after(&map, range.start.min(n));
        let end = map_at_or_after(&map, range.end.min(n));
        if start < end {
            spans.push((start..end, *span));
        }
    }

    proto.debug_info.inst_spans = spans.into();
    proto.instructions = out.into();
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use duka_shared::types::{DebugInfo, SourceName};

    use super::*;

    fn proto(instructions: Vec<Instruction>) -> DukaProto {
        DukaProto {
            up_indexes: Box::new([]),
            constants: Box::new([]),
            runtime_constants: RefCell::new(None),
            instructions: instructions.into_boxed_slice(),
            used_reg_count: 4,
            nested_protos: Box::new([]),
            param_count: 0,
            has_var_arg: false,
            debug_info: Box::new(DebugInfo {
                inst_spans: Box::new([]),
                all_span: Span::EMPTY,
                debug_name: None,
                source_name: SourceName::Unnamed,
                time: None,
            }),
            logic: None,
        }
    }

    fn decoded(proto: &DukaProto) -> Vec<DecodeInstruction> {
        proto
            .instructions
            .iter()
            .map(|inst| inst.decode().unwrap())
            .collect()
    }

    fn optimize(proto: &mut DukaProto) {
        crate::optimizer::optimize(proto, OptimizeConfig::default());
    }

    #[test]
    fn test_and_jump_become_one_instruction() {
        let mut proto = proto(vec![
            Instruction::LoadTrue(0),
            Instruction::Test(0, true),
            Instruction::Jump(2),
            Instruction::LoadNil(0, 1),
            Instruction::Return0(),
        ]);
        optimize(&mut proto);

        let got = decoded(&proto);
        assert_eq!(got.len(), 4);
        assert!(matches!(got[1], DecodeInstruction::TestJump(0, true, 2)));
    }

    #[test]
    fn a_test_keeps_the_instruction_it_skips() {
        // `Move` into itself would normally go, but here the `Test` skips it.
        let mut proto = proto(vec![
            Instruction::Test(0, true),
            Instruction::Move(1, 1),
            Instruction::Return0(),
        ]);
        optimize(&mut proto);

        let got = decoded(&proto);
        assert_eq!(got.len(), 3);
        assert!(matches!(got[1], DecodeInstruction::Move(1, 1)));
    }

    #[test]
    fn write_to_itself_is_dropped() {
        let mut proto = proto(vec![
            Instruction::Move(1, 1),
            Instruction::LoadNil(0, 1),
            Instruction::Return0(),
        ]);
        optimize(&mut proto);

        let got = decoded(&proto);
        assert_eq!(got.len(), 2);
        assert!(matches!(got[0], DecodeInstruction::LoadNil(0, 1)));
    }

    #[test]
    fn a_jump_to_the_next_instruction_is_dropped() {
        let mut proto = proto(vec![
            Instruction::LoadNil(0, 1),
            Instruction::Jump(1),
            Instruction::Return0(),
        ]);
        optimize(&mut proto);

        let got = decoded(&proto);
        assert_eq!(got.len(), 2);
        assert!(matches!(got[1], DecodeInstruction::Return0()));
    }

    #[test]
    fn a_literal_test_that_never_branches_leaves_its_jump() {
        let span = Span::EMPTY;
        let mut proto = proto(vec![
            Instruction::LoadFalse(0),
            Instruction::Test(0, true),
            Instruction::Jump(2),
            Instruction::LoadNil(0, 1),
            Instruction::Return0(),
        ]);
        proto.debug_info.inst_spans = vec![(1..2, span)].into();
        let mut evidence = HashMap::new();
        evidence.insert(span, Arc::new(Type::Literal(ConstValue::Bool(false))));

        crate::optimizer::optimize(
            &mut proto,
            OptimizeConfig {
                evidence: Some(TypeEvidence::new(evidence)),
            },
        );

        // `false == true` fails, so the jump runs: the test goes, the jump
        // stays and still lands on `Return0`.
        let got = decoded(&proto);
        assert_eq!(got.len(), 4);
        assert!(matches!(got[1], DecodeInstruction::Jump(2)));
        assert!(matches!(got[3], DecodeInstruction::Return0()));
    }

    #[test]
    fn a_literal_test_that_always_skips_takes_the_jump_with_it() {
        let span = Span::EMPTY;
        let mut proto = proto(vec![
            Instruction::LoadTrue(0),
            Instruction::Test(0, true),
            Instruction::Jump(2),
            Instruction::LoadNil(0, 1),
            Instruction::Return0(),
        ]);
        proto.debug_info.inst_spans = vec![(1..2, span)].into();
        let mut evidence = HashMap::new();
        evidence.insert(span, Arc::new(Type::Literal(ConstValue::Bool(true))));

        crate::optimizer::optimize(
            &mut proto,
            OptimizeConfig {
                evidence: Some(TypeEvidence::new(evidence)),
            },
        );

        // `true == true` always skips the jump, so both instructions go.
        let got = decoded(&proto);
        assert_eq!(got.len(), 3);
        assert!(matches!(got[1], DecodeInstruction::LoadNil(0, 1)));
    }
}
