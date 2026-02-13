use duka_shared::{
    error::DukaIRError,
    types::{DukaGenerator, Fact, LogicDatabase, Query, Rule},
};

use crate::logic_instructions::LogicInstruction as I;

#[derive(Debug, Clone, PartialEq)]
pub struct LogicProto {}

#[derive(Debug)]
pub struct LogicGenerator {
    instructions: Vec<I>,
}

impl LogicGenerator {
    fn gen_fact(&mut self, Fact(_name, _terms): Fact) -> Result<(), DukaIRError> {
        Ok(())
    }
    fn gen_rule(&mut self, Rule(_name, _terms, _goal): Rule) -> Result<(), DukaIRError> {
        self.instructions.push(I::TRY(0, 10));
        Ok(())
    }
    fn gen_query(&mut self, Query(_goal): Query) -> Result<(), DukaIRError> {
        Ok(())
    }

    fn new() -> Self {
        LogicGenerator {
            instructions: vec![],
        }
    }
    fn gen_logic(mut self, chunk: LogicDatabase) -> Result<LogicProto, DukaIRError> {
        for fact in chunk.facts {
            self.gen_fact(fact)?;
        }
        for rule in chunk.rules {
            self.gen_rule(rule)?;
        }
        for query in chunk.queries.into_vec() {
            self.gen_query(query)?;
        }

        Ok(LogicProto {})
    }
}

impl DukaGenerator<LogicProto> for LogicGenerator {
    type InputType = LogicDatabase;
    fn generate(chunk: Self::InputType) -> Result<LogicProto, DukaIRError> {
        Self::new().gen_logic(chunk)
    }
}
