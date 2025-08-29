use crate::VERSION;
use crate::vm::instructions::Instruction;
use duka_macros::ThatError;
use duka_shared::{
    utils::{OrError, SemVer},
    value::{DukaFloat, DukaInt},
};
use std::io::{Error, Read, Write};

const MAGIC: &[u8; 4] = b"DUKA";
const FLOAT_SIZE: usize = size_of::<DukaFloat>();
const INTEGER_SIZE: usize = size_of::<DukaInt>();
const INSTRUCTION_SIZE: usize = size_of::<Instruction>();
const LITTLE_ENDIAN: bool = true;

pub trait Dumplings: Sized {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError>;
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError>;
}

macro_rules! dumplings {
    (number $ty: ty) => {
        impl Dumplings for $ty {
            fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
                let mut buf = [0u8; size_of::<$ty>()];
                input.read_exact(&mut buf).map_err(DukaDumpError::IOError)?;
                Ok(if LITTLE_ENDIAN {
                    <$ty>::from_le_bytes(buf)
                } else {
                    <$ty>::from_be_bytes(buf)
                })
            }
            fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
                let buf = if LITTLE_ENDIAN {
                    self.to_le_bytes()
                } else {
                    self.to_be_bytes()
                };
                output.write(&buf).map_err(DukaDumpError::IOError)?;
                Ok(())
            }
        }
    };
}
macro_rules! check {
    ($func: expr => $i: ident $op: tt $e: expr, else $err: expr) => {
        (!($func($i)? $op $e)).then_error(|| $err)
    };
}

dumplings!(number u32);
dumplings!(number u16);
dumplings!(number u8);

impl Dumplings for bool {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let mut buf = [0u8];
        input.read_exact(&mut buf).map_err(DukaDumpError::IOError)?;
        Ok(buf[0] == 1)
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        let buf = [*self as u8];
        output.write(&buf).map_err(DukaDumpError::IOError)?;
        Ok(())
    }
}
impl Dumplings for Instruction {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let raw = u32::dl_read(input)?;
        Instruction::validate(raw)
            .then_some(Instruction::from_raw(raw))
            .ok_or(DukaDumpError::UnknownInstruction(raw))
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.raw().dl_write(output)
    }
}
impl Dumplings for SemVer {
    /// # Notice: Neither pre-release nor build message will be recorded
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let major = u8::dl_read(input)?;
        let minor = u8::dl_read(input)?;
        let patch = u8::dl_read(input)?;
        Ok(Self::new(major, minor, patch))
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.major.dl_write(output)?;
        self.minor.dl_write(output)?;
        self.patch.dl_write(output)?;
        Ok(())
    }
}

#[derive(Debug, ThatError)]
pub enum DukaDumpError {
    #[error("IO error: {}")]
    IOError(Error),
    #[error("Unknown instruction read: {}")]
    UnknownInstruction(u32),
    #[error("Found unknown header")]
    UnexpectedMagic,
    #[error("Mismatched {} size: expected {}")]
    MismatchedSize(&'static str, u8),
    #[error("Mismatched endian mode: {} is unsupported")]
    MismatchedEndian(&'static str),
    #[error("Unknown version: {}")]
    UnknownVersion(SemVer),
}

#[derive(Debug, Clone)]
pub struct DukaBinaryHeader;

fn read<const T: usize, R: Read>(input: &mut R) -> Result<[u8; T], DukaDumpError> {
    let mut buf = [0u8; T];
    input.read_exact(&mut buf).map_err(DukaDumpError::IOError)?;
    Ok(buf)
}
fn write<W: Write>(output: &mut W, buf: &[u8]) -> Result<(), DukaDumpError> {
    output.write(buf).map_err(DukaDumpError::IOError)?;
    Ok(())
}

impl Dumplings for DukaBinaryHeader {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        check!(read =>
            input == *MAGIC,
            else DukaDumpError::UnexpectedMagic
        )?;
        check!(bool::dl_read =>
            input == LITTLE_ENDIAN,
            else DukaDumpError::MismatchedEndian(
                LITTLE_ENDIAN
                    .then_some("little endian")
                    .unwrap_or("big endian"))
        )?;
        check!(bool::dl_read =>
            input == LITTLE_ENDIAN,
            else DukaDumpError::MismatchedEndian(
                LITTLE_ENDIAN
                    .then_some("little endian")
                    .unwrap_or("big endian"))
        )?;
        check!(SemVer::dl_read =>
            input <= VERSION,
            else DukaDumpError::UnknownVersion(VERSION)
        )?;
        check!(u8::dl_read =>
            input == INTEGER_SIZE as u8,
            else DukaDumpError::MismatchedSize("integer", INTEGER_SIZE as u8)
        )?;
        check!(u8::dl_read =>
            input == FLOAT_SIZE as u8,
            else DukaDumpError::MismatchedSize("float", FLOAT_SIZE as u8)
        )?;
        check!(u8::dl_read =>
            input == INSTRUCTION_SIZE as u8,
            else DukaDumpError::MismatchedSize("instruction", INSTRUCTION_SIZE as u8)
        )?;

        Ok(Self {})
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        write(output, MAGIC)?;
        LITTLE_ENDIAN.dl_write(output)?;
        VERSION.dl_write(output)?;

        (INTEGER_SIZE as u8).dl_write(output)?;
        (FLOAT_SIZE as u8).dl_write(output)?;
        (INSTRUCTION_SIZE as u8).dl_write(output)?;

        Ok(())
    }
}
