#![allow(dead_code)]

use std::io::{Error, Read, Write};

use duka_macros::ThatError;

use crate::vm::instructions::Instruction;
use duka_shared::{
    utils::OrError,
    value::{DukaFloat, DukaInt},
};

const MAGIC: &[u8; 4] = b"DUKA";
const VERSION: u16 = 1;
const FLOAT_SIZE: usize = size_of::<DukaFloat>();
const INTEGER_SIZE: usize = size_of::<DukaInt>();
const INSTRUCTION_SIZE: usize = size_of::<Instruction>();
const LITTLE_ENDIAN: bool = true;

#[derive(Debug, ThatError)]
pub enum DukaDumpError {
    #[error("IO error: {}")]
    IO(Error),
    #[error("Unknown instruction read: {}")]
    UnknownInstruction(u32),
    #[error("Found unknown header")]
    UnexpectedMagic,
    #[error("Mismatched {} size: expected {}")]
    Size(&'static str, u8),
    #[error("Mismatched endian mode: {} is unsupported")]
    Endian(&'static str),
    #[error("Unknown version: {}")]
    UnknownVersion(u16),
}

struct FileHeader {
    magic: [u8; 4],
    version: u16,
    float_size: usize,
    integer_size: usize,
    instruction_size: usize,
    little_endian: bool,
}

/// read:
/// - magic
/// - endian
/// - version
/// - float size
/// - integer size
/// - instruction size
/// - instructions
/// - constants
/// - protos

fn read<Input: Read>(mut input: Input) -> Result<(), DukaDumpError> {
    check_magic(&mut input)?;
    check_endian(&mut input)?;

    check_size(&mut input, FLOAT_SIZE as u8, "float")?;
    check_size(&mut input, INTEGER_SIZE as u8, "integer")?;
    check_size(&mut input, INSTRUCTION_SIZE as u8, "instruction")?;

    read_instruction(&mut input)?;
    Ok(())
}

fn check_version<Input: Read>(mut input: Input) -> Result<(), DukaDumpError> {
    let version = read_u16(&mut input)?;
    (version > VERSION).then_error(|| DukaDumpError::UnknownVersion(version))
}

fn check_size<Input: Read>(
    mut input: Input,
    be: u8,
    target: &'static str,
) -> Result<(), DukaDumpError> {
    let size = read_u8(&mut input)?;
    (size != be).then_error(|| DukaDumpError::Size(target, be))
}

fn check_magic<Input: Read>(mut input: Input) -> Result<(), DukaDumpError> {
    let mut buf = [0u8; 4];
    input.read(&mut buf).map_err(DukaDumpError::IO)?;
    if buf != *MAGIC {
        Err(DukaDumpError::UnexpectedMagic)
    } else {
        Ok(())
    }
}

fn check_endian<Input: Read>(input: Input) -> Result<(), DukaDumpError> {
    let little_endian = read_bool(input)?;
    (little_endian != LITTLE_ENDIAN).then_error(|| {
        DukaDumpError::Endian(
            little_endian
                .then_some("little endian")
                .unwrap_or("big endian"),
        )
    })
}

fn read_instruction<Input: Read>(input: Input) -> Result<Instruction, DukaDumpError> {
    let raw = read_u32(input)?;
    Instruction::validate(raw)
        .then_some(Instruction::from_raw(raw))
        .ok_or(DukaDumpError::UnknownInstruction(raw))
}

fn read_bool<Input: Read>(mut input: Input) -> Result<bool, DukaDumpError> {
    let mut buf = [0u8; 1];
    input.read_exact(&mut buf).map_err(DukaDumpError::IO)?;
    Ok(buf[0] != 0)
}

macro_rules! binary {
    ($name: ident -> $ty: ty) => {
        fn $name<Input: Read>(mut input: Input) -> Result<$ty, DukaDumpError> {
            let mut buf = [0u8; size_of::<$ty>()];
            input.read_exact(&mut buf).map_err(DukaDumpError::IO)?;
            Ok(if LITTLE_ENDIAN {
                <$ty>::from_le_bytes(buf)
            } else {
                <$ty>::from_be_bytes(buf)
            })
        }
    };
    ($name: ident <- $ty: ty) => {
        fn $name<Output: Write>(mut output: Output, ins: $ty) -> Result<(), DukaDumpError> {
            let buf = if LITTLE_ENDIAN {
                ins.to_le_bytes()
            } else {
                ins.to_be_bytes()
            };
            output.write(&buf).map_err(DukaDumpError::IO)?;
            Ok(())
        }
    };
}

binary!(read_u8 -> u8);
binary!(read_u16 -> u16);
binary!(read_u32 -> u32);

binary!(write_u8 <- u8);
binary!(write_u16 <- u16);
binary!(write_u32 <- u32);

fn write_instruction<Output: Write>(output: Output, ins: Instruction) -> Result<(), DukaDumpError> {
    write_u32(output, ins.raw())?;
    Ok(())
}
