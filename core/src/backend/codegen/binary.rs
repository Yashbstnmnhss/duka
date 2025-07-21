#![allow(dead_code)]

use std::io::{Error, Read, Write};

use duka_macros::ThatError;

use crate::{
    backend::vm::instructions::Instruction,
    shared::value::{DukaFloat, DukaInt},
};

const MAGIC: &[u8; 4] = b"DUKA";
const VERSION: u16 = 1;
const FLOAT_SIZE: usize = size_of::<DukaFloat>();
const INT_SIZE: usize = size_of::<DukaInt>();
const INSTRUCTION_SIZE: usize = size_of::<Instruction>();
const LITTLE_ENDIAN: bool = true;

#[derive(Debug, ThatError)]
pub enum DukaDumpError {
    #[error("IO error: {}")]
    IO(Error),
    #[error("Found unknown instruction: {}")]
    UnknownEndian(u32),
    #[error("Found unknown header")]
    UnexpectMagic,
    #[error("Mismatched endian mode: {} is unsupported")]
    Endian(String),
}

struct FileHeader {
    magic: [u8; 4],
    version: u16,
    float_size: usize,
    int_size: usize,
    instruction_size: usize,
    little_endian: bool,
}

fn read<Input: Read>(mut input: Input) -> Result<(), DukaDumpError> {
    check_magic(&mut input)?;
    check_endian(&mut input)?;
    read_instruction(&mut input)?;
    Ok(())
}

fn check_magic<Input: Read>(mut input: Input) -> Result<(), DukaDumpError> {
    let mut buf = [0u8; 4];
    input.read(&mut buf).map_err(DukaDumpError::IO)?;
    if buf != *MAGIC {
        Err(DukaDumpError::UnexpectMagic)
    } else {
        Ok(())
    }
}

fn check_endian<Input: Read>(input: Input) -> Result<(), DukaDumpError> {
    let little_endian = read_bool(input)?;
    if little_endian == LITTLE_ENDIAN {
        Ok(())
    } else {
        Err(DukaDumpError::Endian(
            if little_endian {
                "little endian"
            } else {
                "big endian"
            }
            .into(),
        ))
    }
}

fn read_instruction<Input: Read>(input: Input) -> Result<Instruction, DukaDumpError> {
    let raw = read_u32(input)?;
    if Instruction::validate(raw) {
        Ok(Instruction::from_raw(raw))
    } else {
        Err(DukaDumpError::UnknownEndian(raw))
    }
}

fn read_bool<Input: Read>(mut input: Input) -> Result<bool, DukaDumpError> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf).map_err(DukaDumpError::IO)?;
    Ok(buf[0] != 0)
}

fn read_u32<Input: Read>(mut input: Input) -> Result<u32, DukaDumpError> {
    let mut buf = [0u8; 4];
    input.read_exact(&mut buf).map_err(DukaDumpError::IO)?;
    Ok(if LITTLE_ENDIAN {
        u32::from_le_bytes(buf)
    } else {
        u32::from_be_bytes(buf)
    })
}

fn write_instruction<Output: Write>(output: Output, ins: Instruction) -> Result<(), DukaDumpError> {
    write_u32(output, ins.raw())?;
    Ok(())
}

fn write_u32<Output: Write>(mut output: Output, ins: u32) -> Result<(), DukaDumpError> {
    let buf = if LITTLE_ENDIAN {
        ins.to_le_bytes()
    } else {
        ins.to_be_bytes()
    };
    output.write(&buf).map_err(DukaDumpError::IO)?;
    Ok(())
}
