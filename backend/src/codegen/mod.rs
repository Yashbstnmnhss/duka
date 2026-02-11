use crate::{
    instructions::{Address, Bits17, DecodeInstruction, Instruction as I},
    value::DukaProto,
};
use duka_shared::error::DukaCodegenError;
use duka_shared::ir::{Allocator, Constants, DukaIR, Scopes};
use duka_shared::types::{DebugInfo, DukaGenerator};

pub mod binary;
pub mod logic;

#[derive(Debug)]
pub struct Generator {
    constants: Constants,
    scopes: Scopes,
    debug_info: DebugInfo,
    instructions: Vec<I>,

    allocator: Allocator,

    nested_protos: Vec<DukaProto>,
}

#[doc = "有可能合并优化的指令"]
impl Generator {
    fn load_nil(&mut self, from: Address, count: Bits17) -> Result<(), DukaCodegenError> {
        if let Some(v) = self.instructions.last_mut()
            && let DecodeInstruction::LoadNil(pfrom, pcount) = v.decode()?
        {
            let (from, pfrom) = (from as u32, pfrom as u32);
            if (pfrom <= from && from <= pfrom + pcount) || (from <= pfrom && pfrom <= from + count)
            {
                // 起点取最小 终点取最大
                let end = (from + count).max(pfrom + pcount) as Bits17;
                let from = from.min(pfrom) as Address;

                *v = I::LoadNil(from, end - (from as Bits17));
                return Ok(());
            }
        }

        //self.emit(I::LoadNil(from, count));
        Ok(())
    }
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator {
    pub fn new() -> Self {
        Self {
            constants: Constants::default(),
            scopes: Scopes::new(),
            allocator: Allocator::new(),
            debug_info: DebugInfo::default(),
            instructions: vec![],
            nested_protos: vec![],
        }
    }
}

impl DukaGenerator<DukaProto> for Generator {
    type InputType = DukaIR;

    fn generate(_ir: Self::InputType) -> Result<DukaProto, DukaCodegenError> {
        // let DukaChunk {
        //     chunk,
        //     span: _,
        //     logic,
        // } = chunk;
        // let logic = LogicGenerator::generate(logic)?;
        // let mut proto = Self::new().generate_proto(chunk, Some("main".to_owned()), None, true)?;
        // proto.logic = Some(logic);
        // Ok(proto)
        Ok(todo!())
    }
}
