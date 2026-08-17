use std::io::{Read, Write};

use duka_lib::codegen::binary::{DukaDumpError, Dump, Load};

pub const MAGIC: &[u8] = b"DUKAB";
pub const FORMAT_VERSION: u8 = 1;
pub const TRAILER_MAGIC: &[u8] = b"DUKABEXE";

fn read<const C: usize, R: Read>(input: &mut R) -> Result<[u8; C], DukaDumpError> {
    let mut buf = [u8::default(); C];
    input.read_exact(&mut buf).map_err(DukaDumpError::IOError)?;
    Ok(buf)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DukaAppHeader;

impl Dump for DukaAppHeader {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        output.write_all(MAGIC).map_err(DukaDumpError::IOError)?;
        FORMAT_VERSION.dump(output)?;
        Ok(())
    }
}
impl Load for DukaAppHeader {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        if &read::<5, _>(input)?[..] != MAGIC {
            return Err(DukaDumpError::UnexpectedMagic);
        }
        let version = u8::load(input)?;
        if version != FORMAT_VERSION {
            return Err(DukaDumpError::UnsupportedFormat(version));
        }
        Ok(Self)
    }
}

#[derive(Debug, Clone)]
pub struct DukaAppBinary {
    header: DukaAppHeader,
    entry: String,
    modules: Vec<(String, Vec<u8>)>,
}

impl Dump for DukaAppBinary {
    fn dump<T: Write>(&self, output: &mut T) -> Result<(), DukaDumpError> {
        self.header.dump(output)?;
        self.entry.dump(output)?;
        self.modules.dump(output)?;
        Ok(())
    }
}
impl Load for DukaAppBinary {
    fn load<T: Read>(input: &mut T) -> Result<Self, DukaDumpError> {
        Ok(Self {
            header: DukaAppHeader::load(input)?,
            entry: String::load(input)?,
            modules: Vec::<(String, Vec<u8>)>::load(input)?,
        })
    }
}

impl DukaAppBinary {
    pub fn new(entry: impl Into<String>, modules: Vec<(String, Vec<u8>)>) -> Self {
        Self {
            header: DukaAppHeader,
            entry: entry.into(),
            modules,
        }
    }
    pub fn entry(&self) -> &str {
        &self.entry
    }
    pub fn modules(&self) -> &[(String, Vec<u8>)] {
        &self.modules
    }
}

pub fn bundle(shell: &[u8], archive: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(shell.len() + archive.len() + TRAILER_MAGIC.len() + 8);
    out.extend_from_slice(shell);
    out.extend_from_slice(archive);
    out.extend_from_slice(&(archive.len() as u64).to_le_bytes());
    out.extend_from_slice(TRAILER_MAGIC);
    out
}

pub fn split(exe: &[u8]) -> Option<(usize, usize)> {
    const LEN: usize = std::mem::size_of::<u64>();
    if exe.len() < TRAILER_MAGIC.len() + LEN {
        return None;
    }
    if &exe[exe.len() - TRAILER_MAGIC.len()..] != TRAILER_MAGIC {
        // Check trailer magic
        return None;
    }
    // Get data length
    let len = u64::from_le_bytes(
        exe[exe.len() - TRAILER_MAGIC.len() - LEN..exe.len() - TRAILER_MAGIC.len()]
            .try_into()
            .ok()?,
    ) as usize;
    if len > exe.len() - TRAILER_MAGIC.len() - LEN {
        return None;
    }
    let start = exe.len() - TRAILER_MAGIC.len() - LEN - len;
    Some((start, len))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn dump_load_roundtrip() {
        let modules = vec![
            ("src/main.duka".to_owned(), vec![1, 2, 3]),
            ("modules/a.duka".to_owned(), vec![4, 5]),
        ];
        let app = DukaAppBinary::new("src/main.duka", modules);
        let mut buf = Vec::new();
        app.dump(&mut buf).unwrap();
        let loaded = DukaAppBinary::load(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(loaded.entry(), "src/main.duka");
        assert_eq!(
            loaded.modules(),
            &[
                ("src/main.duka".to_owned(), vec![1, 2, 3]),
                ("modules/a.duka".to_owned(), vec![4, 5]),
            ]
        );
    }

    #[test]
    fn bad_magic_rejected() {
        let mut buf = Vec::new();
        b"WRONG".as_slice().dump(&mut buf).unwrap();
        assert!(DukaAppBinary::load(&mut Cursor::new(&buf)).is_err());
    }

    #[test]
    fn bundle_split_roundtrip() {
        let shell = vec![1, 2, 3, 4];
        let archive = vec![9, 9, 9];
        let exe = bundle(&shell, &archive);
        assert_eq!(&exe[..shell.len()], &shell[..]);
        let (start, len) = split(&exe).unwrap();
        assert_eq!(len, archive.len());
        assert_eq!(&exe[start..start + len], &archive[..]);
    }

    #[test]
    fn split_rejects_plain_file() {
        assert!(split(b"not an application").is_none());
    }

    #[test]
    fn split_rejects_tampered_len() {
        let shell = vec![1, 2, 3];
        let archive = vec![9, 9, 9, 9];
        let mut exe = bundle(&shell, &archive);
        let n = exe.len() - TRAILER_MAGIC.len() - std::mem::size_of::<u64>();
        exe[n] = 0xFF;
        exe[n + 1] = 0xFF;
        exe[n + 2] = 0xFF;
        assert!(split(&exe).is_none());
    }
}
