use crate::DebugInfo;
use crate::codegen::logic::LogicProto;
use crate::instructions::Instruction;
use crate::value::{UpIndex, UpValueKind};
use crate::{VERSION, value::DukaProto};
use duka_macros::ThatError;
use duka_shared::error::{Position, Span};
use duka_shared::value::{ArrayMap, ConstValue};
use duka_shared::{
    utils::{OrError, SemVer},
    value::{DukaFloat, DukaInt},
};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{Error, Read, Write};
use std::ops::Range;
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
impl<A, B> Dumplings for (A, B)
where
    A: Dumplings,
    B: Dumplings,
{
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok((A::dl_read(input)?, B::dl_read(input)?))
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.0.dl_write(output)?;
        self.1.dl_write(output)?;
        Ok(())
    }
}

impl<K: Dumplings + Hash + Eq, V: Dumplings> Dumplings for HashMap<K, V> {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let count = usize::dl_read(input)?;
        let mut map = HashMap::with_capacity(count);
        for _ in 0..count {
            let (k, v) = <(K, V)>::dl_read(input)?;
            map.insert(k, v);
        }
        Ok(map)
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dl_write(output)?;
        for (k, v) in self {
            k.dl_write(output)?;
            v.dl_write(output)?;
        }
        Ok(())
    }
}

impl<V: Dumplings> Dumplings for Vec<V> {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let len = usize::dl_read(input)?;
        let mut temp = vec![];
        temp.reserve(len);
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

impl Dumplings for ConstValue {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        use ConstValue::*;

        let tag = ConstValue::disc2name(u8::dl_read(input)?)
            .map_err(|g| UnknownDiscriminant(g, "ConstValue"))?;
        Ok(match tag {
            "nil" => Nil,
            "int" => Int(DukaInt::dl_read(input)?),
            "float" => Float(DukaFloat::dl_read(input)?),
            "bool" => Bool(bool::dl_read(input)?),
            "consttable" => {
                // table: read array then map
                let mut am = ArrayMap::new();
                am.inner = HashMap::<ConstValue, ConstValue>::dl_read(input)?;
                ConstTable(am)
            }
            "string" => String(Vec::<u8>::dl_read(input)?),
            _ => unreachable!(),
        })
    }

    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        use ConstValue::*;

        self.disc().dl_write(output)?;
        match self {
            Nil => (),
            Int(i) => i.dl_write(output)?,
            Float(f) => f.dl_write(output)?,
            Bool(b) => b.dl_write(output)?,
            ConstTable(rc) => {
                rc.inner.dl_write(output)?;
            }
            String(b) => b.dl_write(output)?,
        }
        Ok(())
    }
}
impl Dumplings for UpValueKind {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self::from_disc(u8::dl_read(input)?)
            .ok()
            .unwrap_or_default())
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.disc().dl_write(output)?;
        Ok(())
    }
}
impl Dumplings for UpIndex {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(UpIndex {
            name: Option::<String>::dl_read(input)?,
            local: bool::dl_read(input)?,
            index: usize::dl_read(input)?,
            kind: UpValueKind::dl_read(input)?,
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.name.dl_write(output)?;
        self.local.dl_write(output)?;
        self.index.dl_write(output)?;
        self.kind.dl_write(output)?;
        Ok(())
    }
}

#[derive(Debug, ThatError)]
pub enum DukaDumpError {
    #[error("IO error: {}")]
    IOError(Error),
    #[error("Unknown discriminant {} for {}")]
    UnknownDiscriminant(u8, &'static str),
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
    #[error("Cannot dump runtime value in {}")]
    CannotDumpRuntimeValue(&'static str),
}
use DukaDumpError::*;

#[derive(Debug, Clone, PartialEq)]
pub struct DukaBinaryHeader;

fn read<const C: usize, R: Read>(input: &mut R) -> Result<[u8; C], DukaDumpError> {
    let mut buf = [u8::default(); C];
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
            input >= VERSION,
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

impl Dumplings for LogicProto {
    fn dl_read<T: Read>(_input: &mut T) -> Result<Self, DukaDumpError> {
        todo!()
    }
    fn dl_write<T: Write>(&self, _output: &mut T) -> Result<(), DukaDumpError> {
        Ok(())
    }
}

impl Dumplings for Position {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            line: usize::dl_read(input)?,
            column: usize::dl_read(input)?,
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.line.dl_write(output)?;
        self.column.dl_write(output)?;
        Ok(())
    }
}

impl Dumplings for Span {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            start: Position::dl_read(input)?,
            end: Position::dl_read(input)?,
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.start.dl_write(output)?;
        self.end.dl_write(output)?;
        Ok(())
    }
}

impl<V: Dumplings> Dumplings for Range<V> {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(V::dl_read(input)?..V::dl_read(input)?)
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.start.dl_write(output)?;
        self.end.dl_write(output)?;
        Ok(())
    }
}

impl Dumplings for DebugInfo {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            all_span: Span::dl_read(input)?,
            debug_name: Option::<String>::dl_read(input)?,
            inst_spans: HashMap::<_, _>::dl_read(input)?,
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.all_span.dl_write(output)?;
        self.debug_name.dl_write(output)?;
        self.inst_spans.dl_write(output)?;
        Ok(())
    }
}

impl Dumplings for DukaProto {
    fn dl_read<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let debug_info = DebugInfo::dl_read(input)?;
        let has_var_arg = bool::dl_read(input)?;
        let param_count = usize::dl_read(input)?;
        let reg_count = usize::dl_read(input)?;
        let instructions = Vec::<Instruction>::dl_read(input)?;
        let upvalues = Vec::<UpIndex>::dl_read(input)?;
        let constants = Vec::<ConstValue>::dl_read(input)?;
        let nested_protos = Vec::<DukaProto>::dl_read(input)?;
        let logic = Option::<LogicProto>::dl_read(input)?;

        Ok(Self {
            up_indexes: upvalues,
            constants,
            instructions,
            nested_protos,
            param_count,
            reg_count,
            has_var_arg,
            debug_info,
            logic,
        })
    }
    fn dl_write<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.debug_info.dl_write(output)?;

        self.has_var_arg.dl_write(output)?;
        self.param_count.dl_write(output)?;

        self.reg_count.dl_write(output)?;

        self.instructions.dl_write(output)?;
        self.up_indexes.dl_write(output)?;
        // write constants and nested protos
        self.constants.dl_write(output)?;
        self.nested_protos.dl_write(output)?;

        self.logic.dl_write(output)?;

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
