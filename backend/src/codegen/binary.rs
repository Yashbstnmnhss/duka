use crate::instructions::Instruction;
use crate::{VERSION, value::DukaProto, value::RuntimeValue};
use duka_macros::ThatError;
use duka_shared::value::ConstValue;
use duka_shared::value::SHORT_STR_LEN;
use duka_shared::{
    utils::{OrError, SemVer},
    value::{DukaFloat, DukaInt},
};
use std::io::{Error, Read, Write};
use std::string::FromUtf8Error;

const FORMAT_VERSION: u8 = 0;
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
    ($func: expr => $i: ident $op: tt $e: expr, else $err: expr) => {{
        let val = $func($i)?;
        (!(val $op $e)).then_error(|| $err(val))
    }};
}

dumplings!(number DukaInt);
dumplings!(number DukaFloat);

dumplings!(number usize);
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

impl<V: Dumplings> Dumplings for Option<V> {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(if bool::dl_read(input)? {
            Some(V::dl_read(input)?)
        } else {
            None
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        if let Some(val) = self {
            true.dl_write(output)?;
            val.dl_write(output)?;
        } else {
            false.dl_write(output)?;
        }
        Ok(())
    }
}
impl Dumplings for String {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let len = usize::dl_read(input)?;
        let mut buffer = vec![u8::default(); len];
        input
            .read_exact(&mut buffer)
            .map_err(DukaDumpError::IOError)?;
        Ok(String::from_utf8(buffer).map_err(DukaDumpError::InvalidUTF8)?)
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        let bytes = self.as_bytes();
        bytes.len().dl_write(output)?;
        output.write_all(bytes).map_err(DukaDumpError::IOError)?;
        Ok(())
    }
}
impl<V: Dumplings> Dumplings for Vec<V> {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let len = usize::dl_read(input)?;
        let mut temp = vec![];
        for _ in 0..len {
            temp.push(V::dl_read(input)?);
        }
        Ok(temp)
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dl_write(output)?;
        for item in self {
            item.dl_write(output)?;
        }
        Ok(())
    }
}

impl Dumplings for RuntimeValue {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        use RuntimeValue::*;
        let disc = usize::dl_read(input)?;
        Ok(match RuntimeValue::discrimination2name(disc) {
            "nil" => Nil,
            "int" => Int(DukaInt::dl_read(input)?),
            "float" => Float(DukaFloat::dl_read(input)?),
            "bool" => Bool(bool::dl_read(input)?),
            "shortstring" => {
                let len = u8::dl_read(input)?;
                let mut buffer = [u8::default(); SHORT_STR_LEN];
                input
                    .read_exact(&mut buffer)
                    .map_err(DukaDumpError::IOError)?;
                ShortString(len, buffer)
            }
            "coroutine" => Coroutine(usize::dl_read(input)?),
            _ => unimplemented!(),
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        use RuntimeValue::*;
        self.discrimination().dl_write(output)?;
        match self {
            Nil => (),
            Int(n) => n.dl_write(output)?,
            Float(n) => n.dl_write(output)?,
            Bool(b) => b.dl_write(output)?,
            ShortString(len, bytes) => {
                len.dl_write(output)?;
                output.write_all(bytes).map_err(DukaDumpError::IOError)?;
            }
            Coroutine(id) => id.dl_write(output)?,
            MediumString(gc) => todo!(),
            LongString(gc) => todo!(),
            Table(gc) => todo!(),
            UserData() => todo!(),
            LightUserData() => todo!(),
            UserFunc(gc) => todo!(),
            NativeFunc(gc) => todo!(),
        }
        Ok(())
    }
}

impl Dumplings for ConstValue {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let tag = u8::dl_read(input)?;
        match tag {
            0 => Ok(ConstValue::Nil),
            1 => Ok(ConstValue::Int(DukaInt::dl_read(input)?)),
            2 => Ok(ConstValue::Float(DukaFloat::dl_read(input)?)),
            3 => Ok(ConstValue::Bool(bool::dl_read(input)?)),
            4 => {
                // table: read array then map
                let arr: Vec<ConstValue> = Vec::<ConstValue>::dl_read(input)?;
                let map_len = usize::dl_read(input)?;
                let mut am = duka_shared::value::ArrayMap::new();
                am.array = arr;
                for _ in 0..map_len {
                    let k = ConstValue::dl_read(input)?;
                    let v = ConstValue::dl_read(input)?;
                    am.map.insert(k, v);
                }
                Ok(ConstValue::ConstTable(std::rc::Rc::new(
                    std::cell::RefCell::new(am),
                )))
            }
            5 => {
                let s = Vec::<u8>::dl_read(input)?;
                Ok(ConstValue::String(s))
            }
            _ => Err(DukaDumpError::UnexpectedMagic),
        }
    }

    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        match self {
            ConstValue::Nil => {
                0u8.dl_write(output)?;
            }
            ConstValue::Int(i) => {
                1u8.dl_write(output)?;
                i.dl_write(output)?;
            }
            ConstValue::Float(f) => {
                2u8.dl_write(output)?;
                f.dl_write(output)?;
            }
            ConstValue::Bool(b) => {
                3u8.dl_write(output)?;
                b.dl_write(output)?;
            }
            ConstValue::ConstTable(rc) => {
                4u8.dl_write(output)?;
                let borrowed = rc.borrow();
                // write array
                borrowed.array.dl_write(output)?;
                // write map as length + pairs
                let pairs_len = borrowed.map.len();
                pairs_len.dl_write(output)?;
                for (k, v) in &borrowed.map {
                    k.dl_write(output)?;
                    v.dl_write(output)?;
                }
            }
            ConstValue::String(b) => {
                5u8.dl_write(output)?;
                b.dl_write(output)?;
            }
        }
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
    #[error("Mismatched {} size: expected {}, got {}")]
    MismatchedSize(&'static str, u8, u8),
    #[error("Mismatched endian mode: {} is unsupported")]
    MismatchedEndian(&'static str),
    #[error("Unknown version: {}")]
    UnknownVersion(SemVer),
    #[error("Unsupported format: {}")]
    UnsupportedFormat(u8),
    #[error("Failed to read UTF-8 string: {}")]
    InvalidUTF8(FromUtf8Error),
}
use DukaDumpError::*;

#[derive(Debug, Clone, PartialEq)]
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
            else |_| UnexpectedMagic
        )?;
        check!(bool::dl_read =>
            input == LITTLE_ENDIAN,
            else |_| MismatchedEndian(
                LITTLE_ENDIAN
                    .then_some("little endian")
                    .unwrap_or("big endian"))
        )?;
        check!(u8::dl_read =>
            input == FORMAT_VERSION,
            else UnsupportedFormat
        )?;
        check!(SemVer::dl_read =>
            input > VERSION,
            else |_| UnknownVersion(VERSION)
        )?;
        check!(u8::dl_read =>
            input == INTEGER_SIZE as u8,
            else |val| MismatchedSize("integer", INTEGER_SIZE as u8, val)
        )?;
        check!(u8::dl_read =>
            input == FLOAT_SIZE as u8,
            else |val| MismatchedSize("float", FLOAT_SIZE as u8, val)
        )?;
        check!(u8::dl_read =>
            input == INSTRUCTION_SIZE as u8,
            else |val| MismatchedSize("instruction", INSTRUCTION_SIZE as u8, val)
        )?;

        Ok(Self)
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        write(output, MAGIC)?;
        LITTLE_ENDIAN.dl_write(output)?;
        FORMAT_VERSION.dl_write(output)?;
        VERSION.dl_write(output)?;

        (INTEGER_SIZE as u8).dl_write(output)?;
        (FLOAT_SIZE as u8).dl_write(output)?;
        (INSTRUCTION_SIZE as u8).dl_write(output)?;

        Ok(())
    }
}

impl Dumplings for DukaProto {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        // Ok(Self {
        //     debug_name: Option::<String>::dl_read(input)?,
        //     constants: todo!(),
        // })
        let debug_name = Option::<String>::dl_read(input)?;
        let has_var_arg = bool::dl_read(input)?;
        let param_count = usize::dl_read(input)?;

        let instructions = Vec::<Instruction>::dl_read(input)?;
        let constants = Vec::<ConstValue>::dl_read(input)?;
        let nested_protos = Vec::<DukaProto>::dl_read(input)?;

        Ok(Self {
            upvalues: vec![],
            constants,
            instructions,
            nested_protos,
            param_count,
            has_var_arg,
            debug_name,
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.debug_name.dl_write(output)?;

        self.has_var_arg.dl_write(output)?;
        self.param_count.dl_write(output)?;

        self.instructions.dl_write(output)?;

        // write constants and nested protos
        self.constants.dl_write(output)?;
        self.nested_protos.dl_write(output)?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaBinary {
    header: DukaBinaryHeader,
    proto: DukaProto,
}

impl Dumplings for DukaBinary {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            header: DukaBinaryHeader::dl_read(input)?,
            proto: DukaProto::dl_read(input)?,
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.header.dl_write(output)?;
        self.proto.dl_write(output)?;
        Ok(())
    }
}
impl DukaBinary {
    pub fn new(proto: DukaProto) -> Self {
        Self {
            header: DukaBinaryHeader,
            proto,
        }
    }
}
