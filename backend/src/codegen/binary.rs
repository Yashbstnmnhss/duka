use crate::codegen::logic::{CompiledQuery, LogicProto, Procedure};
use crate::instructions::{Instruction, logic::LogicInstruction};
use crate::{VERSION, value::DukaProto};
use duka_macros::ThatError;
use duka_shared::errors::{Position, Span};
use duka_shared::ir::{UpIndex, UpValueKind};
use duka_shared::types::DebugInfo;
use duka_shared::types::{QueryCount, SourceInfo};
use duka_shared::value::{ArrayMap, ConstValue};
use duka_shared::{
    utils::{OrError, SemVer},
    value::{DukaFloat, DukaInt},
};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{Error, Read, Write};
use std::ops::Range;
use std::str::Utf8Error;
use std::string::FromUtf8Error;
use std::sync::Arc;
use std::time::Instant;

const FORMAT_VERSION: u8 = 1;
const MAGIC: &[u8; 4] = b"DUKA";
const FLOAT_SIZE: usize = size_of::<DukaFloat>();
const INTEGER_SIZE: usize = size_of::<DukaInt>();
const INSTRUCTION_SIZE: usize = size_of::<Instruction>();
const LITTLE_ENDIAN: bool = true;

pub trait Dump: Sized {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError>;
}
pub trait Load: Sized {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError>;
}

macro_rules! dumplings {
    (number $ty: ty) => {
        impl Dump for $ty {
            fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
                let buf = if LITTLE_ENDIAN {
                    self.to_le_bytes()
                } else {
                    self.to_be_bytes()
                };
                output.write(&buf).map_err(DukaDumpError::IOError)?;
                Ok(())
            }
        }
        impl Load for $ty {
            fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
                let mut buf = [0u8; size_of::<$ty>()];
                input.read_exact(&mut buf).map_err(DukaDumpError::IOError)?;
                Ok(if LITTLE_ENDIAN {
                    <$ty>::from_le_bytes(buf)
                } else {
                    <$ty>::from_be_bytes(buf)
                })
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

impl Dump for bool {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        let buf = [*self as u8];
        output.write(&buf).map_err(IOError)?;
        Ok(())
    }
}
impl Load for bool {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let mut buf = [0u8];
        input.read_exact(&mut buf).map_err(IOError)?;
        Ok(buf[0] == 1)
    }
}

impl Dump for Instruction {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.raw().dump(output)
    }
}
impl Load for Instruction {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let raw = u32::load(input)?;
        Instruction::validate(raw)
            .then_some(Instruction::from_raw(raw))
            .ok_or(UnknownInstruction(raw))
    }
}

impl Dump for SemVer {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.major.dump(output)?;
        self.minor.dump(output)?;
        self.patch.dump(output)?;
        Ok(())
    }
}
impl Load for SemVer {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let major = u8::load(input)?;
        let minor = u8::load(input)?;
        let patch = u8::load(input)?;
        Ok(Self::new(major, minor, patch))
    }
}

impl<V: Dump> Dump for Option<V> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        if let Some(val) = self {
            true.dump(output)?;
            val.dump(output)?;
        } else {
            false.dump(output)?;
        }
        Ok(())
    }
}
impl<V: Load> Load for Option<V> {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(if bool::load(input)? {
            Some(V::load(input)?)
        } else {
            None
        })
    }
}

impl Dump for String {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        let bytes = self.as_bytes();
        bytes.len().dump(output)?;
        output.write_all(bytes).map_err(IOError)?;
        Ok(())
    }
}
impl Load for String {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let len = usize::load(input)?;
        let mut buffer = vec![u8::default(); len];
        input.read_exact(&mut buffer).map_err(IOError)?;
        String::from_utf8(buffer).map_err(InvalidUTF8)
    }
}

impl<A: Dump, B: Dump> Dump for (A, B) {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.0.dump(output)?;
        self.1.dump(output)?;
        Ok(())
    }
}
impl<A: Load, B: Load> Load for (A, B) {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok((A::load(input)?, B::load(input)?))
    }
}

impl<K: Dump + Hash + Eq, V: Dump> Dump for HashMap<K, V> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dump(output)?;
        for (k, v) in self {
            k.dump(output)?;
            v.dump(output)?;
        }
        Ok(())
    }
}
impl<K: Load + Hash + Eq, V: Load> Load for HashMap<K, V> {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let count = usize::load(input)?;
        let mut map = HashMap::with_capacity(count);
        for _ in 0..count {
            let (k, v) = <(K, V)>::load(input)?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

impl<V: Dump> Dump for Vec<V> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dump(output)?;
        for item in self {
            item.dump(output)?;
        }
        Ok(())
    }
}
impl<V: Load> Load for Vec<V> {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let len = usize::load(input)?;
        let mut temp = Vec::with_capacity(len);
        for _ in 0..len {
            temp.push(V::load(input)?);
        }
        Ok(temp)
    }
}

impl Dump for ConstValue {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        use ConstValue::*;
        self.disc().dump(output)?;
        match self {
            Nil => (),
            Int(i) => i.dump(output)?,
            Float(f) => f.dump(output)?,
            Bool(b) => b.dump(output)?,
            ConstTable(rc) => {
                rc.inner.dump(output)?;
            }
            String(b) => b.dump(output)?,
        }
        Ok(())
    }
}
impl Load for ConstValue {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        use ConstValue::*;
        let tag = ConstValue::disc2name(u8::load(input)?)
            .map_err(|g| UnknownDiscriminant(g, "ConstValue"))?;
        Ok(match tag {
            "nil" => Nil,
            "int" => Int(DukaInt::load(input)?),
            "float" => Float(DukaFloat::load(input)?),
            "bool" => Bool(bool::load(input)?),
            "consttable" => {
                let mut am = ArrayMap::new();
                am.inner = HashMap::<ConstValue, ConstValue>::load(input)?;
                ConstTable(Box::new(am))
            }
            "string" => String(Box::<[u8]>::load(input)?),
            _ => unreachable!(),
        })
    }
}

impl Dump for UpValueKind {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.disc().dump(output)?;
        Ok(())
    }
}
impl Load for UpValueKind {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self::from_disc(u8::load(input)?).ok().unwrap_or_default())
    }
}

impl Dump for UpIndex {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.name.dump(output)?;
        self.local.dump(output)?;
        self.index.dump(output)?;
        self.kind.dump(output)?;
        Ok(())
    }
}
impl Load for UpIndex {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(UpIndex {
            name: Option::<String>::load(input)?,
            local: bool::load(input)?,
            index: usize::load(input)?,
            kind: UpValueKind::load(input)?,
        })
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
    #[error("Failed to read UTF-8 string: {}")]
    InvalidUTF8Str(Utf8Error),
    #[error("Cannot dump runtime value in {}")]
    CannotDumpRuntimeValue(&'static str),
}
use DukaDumpError::*;

#[derive(Debug, Clone, PartialEq)]
pub struct DukaBinaryHeader;

fn read<const C: usize, R: Read>(input: &mut R) -> Result<[u8; C], DukaDumpError> {
    let mut buf = [u8::default(); C];
    input.read_exact(&mut buf).map_err(IOError)?;
    Ok(buf)
}
fn write<W: Write>(output: &mut W, buf: &[u8]) -> Result<(), DukaDumpError> {
    output.write(buf).map_err(IOError)?;
    Ok(())
}

impl Dump for DukaBinaryHeader {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        write(output, MAGIC)?;
        LITTLE_ENDIAN.dump(output)?;
        FORMAT_VERSION.dump(output)?;
        VERSION.dump(output)?;
        (INTEGER_SIZE as u8).dump(output)?;
        (FLOAT_SIZE as u8).dump(output)?;
        (INSTRUCTION_SIZE as u8).dump(output)?;
        Ok(())
    }
}
impl Load for DukaBinaryHeader {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        check!(read =>
            input == *MAGIC,
            else |_| UnexpectedMagic
        )?;
        check!(bool::load =>
            input == LITTLE_ENDIAN,
            else |_| MismatchedEndian(
                "little endian")
        )?;
        check!(u8::load =>
            input == FORMAT_VERSION,
            else UnsupportedFormat
        )?;
        check!(SemVer::load =>
            input >= VERSION,
            else |_| UnknownVersion(VERSION)
        )?;
        check!(u8::load =>
            input == INTEGER_SIZE as u8,
            else |val| MismatchedSize("integer", INTEGER_SIZE as u8, val)
        )?;
        check!(u8::load =>
            input == FLOAT_SIZE as u8,
            else |val| MismatchedSize("float", FLOAT_SIZE as u8, val)
        )?;
        check!(u8::load =>
            input == INSTRUCTION_SIZE as u8,
            else |val| MismatchedSize("instruction", INSTRUCTION_SIZE as u8, val)
        )?;
        Ok(Self)
    }
}

impl Dump for LogicInstruction {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.raw().dump(output)
    }
}
impl Load for LogicInstruction {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let raw = u32::load(input)?;
        LogicInstruction::validate(raw)
            .then_some(LogicInstruction::from_raw(raw))
            .ok_or(UnknownInstruction(raw))
    }
}

impl Dump for QueryCount {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        match self {
            QueryCount::Binding(s) => {
                0u8.dump(output)?;
                s.dump(output)?;
            }
            QueryCount::Exact(n) => {
                1u8.dump(output)?;
                n.dump(output)?;
            }
            QueryCount::All => {
                2u8.dump(output)?;
            }
        }
        Ok(())
    }
}
impl Load for QueryCount {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(match u8::load(input)? {
            0 => QueryCount::Binding(String::load(input)?),
            1 => QueryCount::Exact(usize::load(input)?),
            2 => QueryCount::All,
            d => return Err(UnknownDiscriminant(d, "QueryCount")),
        })
    }
}

impl Dump for CompiledQuery {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.instructions.dump(output)?;
        self.count.dump(output)?;
        Ok(())
    }
}
impl Load for CompiledQuery {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            instructions: Vec::<LogicInstruction>::load(input)?,
            count: QueryCount::load(input)?,
        })
    }
}

impl Dump for Procedure {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.name.dump(output)?;
        self.arity.dump(output)?;
        self.clauses.dump(output)?;
        Ok(())
    }
}
impl Load for Procedure {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            name: String::load(input)?,
            arity: usize::load(input)?,
            clauses: Vec::<Vec<LogicInstruction>>::load(input)?,
        })
    }
}

impl Dump for LogicProto {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.procedures.dump(output)?;
        self.queries.dump(output)?;
        self.strings.to_slice().dump(output)?;
        Ok(())
    }
}
impl Load for LogicProto {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            procedures: Vec::<Procedure>::load(input)?,
            queries: Vec::<CompiledQuery>::load(input)?,
            strings: Vec::<String>::load(input)?.into(),
        })
    }
}

impl<S: Dump> Dump for &[S] {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dump(output)?;
        for v in *self {
            v.dump(output)?;
        }
        Ok(())
    }
}

impl Dump for Position {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.line.dump(output)?;
        self.column.dump(output)?;
        self.at_char.dump(output)?;
        Ok(())
    }
}
impl Load for Position {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            line: u32::load(input)?,
            column: u32::load(input)?,
            at_char: u32::load(input)?,
        })
    }
}

impl Dump for Span {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.start.dump(output)?;
        self.end.dump(output)?;
        Ok(())
    }
}
impl Load for Span {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            start: Position::load(input)?,
            end: Position::load(input)?,
        })
    }
}

impl<V: Dump> Dump for Range<V> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.start.dump(output)?;
        self.end.dump(output)?;
        Ok(())
    }
}
impl<V: Load> Load for Range<V> {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(V::load(input)?..V::load(input)?)
    }
}

impl Dump for &str {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dump(output)?;
        output.write_all(self.as_bytes()).map_err(IOError)?;
        Ok(())
    }
}

impl Dump for Box<str> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dump(output)?;
        output.write_all(self.as_bytes()).map_err(IOError)?;
        Ok(())
    }
}
impl Dump for Arc<str> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dump(output)?;
        output.write_all(self.as_bytes()).map_err(IOError)?;
        Ok(())
    }
}
impl Load for Arc<str> {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Arc::from(String::load(input)?))
    }
}

impl Dump for SourceInfo {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.name.dump(output)?;
        self.source.to_vec().dump(output)?;
        Ok(())
    }
}
impl Load for SourceInfo {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(SourceInfo {
            name: Option::<Arc<str>>::load(input)?,
            source: Vec::<u8>::load(input)?.into(),
            time: Instant::now(),
        })
    }
}

impl Dump for DebugInfo {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.all_span.dump(output)?;
        self.debug_name.dump(output)?;
        self.inst_spans.dump(output)?;
        self.source_info.dump(output)?;
        Ok(())
    }
}
impl Load for DebugInfo {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            all_span: Span::load(input)?,
            debug_name: Option::<String>::load(input)?.map(|s| s.into_boxed_str()),
            inst_spans: Vec::<(_, _)>::load(input)?.into(),
            source_info: SourceInfo::load(input)?,
        })
    }
}

impl<V: Dump> Dump for Box<V> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        (**self).dump(output)?;
        Ok(())
    }
}
impl<V: Load> Load for Box<V> {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Box::new(V::load(input)?))
    }
}

impl<V: Dump> Dump for Box<[V]> {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.len().dump(output)?;
        for el in self {
            el.dump(output)?;
        }
        Ok(())
    }
}
impl<V: Load> Load for Box<[V]> {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let el = Vec::<V>::load(input)?;
        Ok(el.into())
    }
}

impl Dump for DukaProto {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.debug_info.dump(output)?;
        self.has_var_arg.dump(output)?;
        self.param_count.dump(output)?;
        self.used_reg_count.dump(output)?;
        self.instructions.dump(output)?;
        self.up_indexes.dump(output)?;
        self.constants.dump(output)?;
        self.nested_protos.dump(output)?;
        self.logic.dump(output)?;
        Ok(())
    }
}
impl Load for DukaProto {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        let debug_info = Box::<DebugInfo>::load(input)?;
        let has_var_arg = bool::load(input)?;
        let param_count = usize::load(input)?;
        let used_reg_count = usize::load(input)?;
        let instructions = Box::<[Instruction]>::load(input)?;
        let up_values = Box::<[UpIndex]>::load(input)?;
        let constants = Box::<[ConstValue]>::load(input)?;
        let nested_protos = Box::<[DukaProto]>::load(input)?;
        let logic = Option::<Box<LogicProto>>::load(input)?;
        Ok(Self {
            up_indexes: up_values,
            constants,
            runtime_constants: std::cell::RefCell::new(None),
            instructions,
            nested_protos,
            param_count,
            used_reg_count,
            has_var_arg,
            debug_info,
            logic,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaBinary {
    header: DukaBinaryHeader,
    proto: DukaProto,
}

impl Dump for DukaBinary {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.header.dump(output)?;
        self.proto.dump(output)?;
        Ok(())
    }
}
impl Load for DukaBinary {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            header: DukaBinaryHeader::load(input)?,
            proto: DukaProto::load(input)?,
        })
    }
}
impl DukaBinary {
    pub fn new(proto: DukaProto) -> Self {
        Self {
            header: DukaBinaryHeader,
            proto,
        }
    }
    pub fn into_proto(self) -> DukaProto {
        self.proto
    }
}
