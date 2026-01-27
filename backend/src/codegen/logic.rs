use duka_shared::{
    error::DukaCodegenError,
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
    fn gen_fact(&mut self, Fact(name, terms): Fact) -> Result<(), DukaCodegenError> {
        Ok(())
    }
    fn gen_rule(&mut self, Rule(name, terms, goal): Rule) -> Result<(), DukaCodegenError> {
        self.instructions.push(I::TRY(0, 10));
        Ok(())
    }
    fn gen_query(&mut self, Query(goal): Query) -> Result<(), DukaCodegenError> {
        Ok(())
    }

    fn new() -> Self {
        LogicGenerator {
            instructions: vec![],
        }
    }
    fn gen_logic(mut self, chunk: LogicDatabase) -> Result<LogicProto, DukaCodegenError> {
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
    fn generate(chunk: Self::InputType) -> Result<LogicProto, DukaCodegenError> {
        Self::new().gen_logic(chunk)
    }
}
