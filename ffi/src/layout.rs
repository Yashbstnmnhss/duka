use std::collections::{HashMap, HashSet};

use crate::cdef::{CBaseType, CDecls, CEnum, CStruct, CTag, CType, CUnion};

pub const LONG_SIZE: usize = if cfg!(target_os = "windows") { 4 } else { 8 };
pub const LONG_DOUBLE_SIZE: usize = if cfg!(any(
    all(target_os = "windows", target_pointer_width = "64"),
    all(target_os = "macos", target_pointer_width = "64"),
    all(target_os = "windows", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "aarch64")
)) {
    8
} else if cfg!(all(target_arch = "x86", not(target_os = "macos"))) {
    12
} else {
    16
};
pub const PTR_SIZE: usize = std::mem::size_of::<usize>();

#[inline]
pub const fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}

#[inline]
pub fn enum_info(e: &CEnum, target: &Target) -> Option<TypeInfo> {
    if e.1.is_empty() {
        return None;
    }

    let mut next = 0isize;
    let mut min = 0isize;
    let mut max = 0isize;

    for (_, n) in e.1.iter() {
        let v = n.unwrap_or(next);
        min = min.min(v);
        max = max.max(v);
        next = v + 1;
    }

    Some(if min >= i32::MIN as isize && max <= i32::MAX as isize {
        target.int
    } else if min >= 0 && max <= u32::MAX as isize {
        target.int
    } else if min >= i64::MIN as isize && max <= i64::MAX as isize {
        target.long_long
    } else {
        unreachable!("Enum too large")
    })
}

#[inline]
pub const fn base_info(ty: &CBaseType, target: &Target) -> TypeInfo {
    match ty {
        CBaseType::Void => target.void,
        CBaseType::Bool => target.bool,
        CBaseType::Char(_) => target.char,
        CBaseType::Short(_) => target.short,
        CBaseType::Int(_) => target.int,
        CBaseType::Long(_) => target.long,
        CBaseType::LongLong(_) => target.long_long,
        CBaseType::Float => target.float,
        CBaseType::Double => target.double,
        CBaseType::LongDouble => target.long_double,
    }
}

pub type TypeInfo = (usize, usize);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub void: TypeInfo,
    pub bool: TypeInfo,
    pub char: TypeInfo,
    pub short: TypeInfo,
    pub int: TypeInfo,
    pub long: TypeInfo,
    pub long_long: TypeInfo,
    pub float: TypeInfo,
    pub double: TypeInfo,
    pub long_double: TypeInfo,
    pub ptr: TypeInfo,
}
impl Target {
    pub const HOST: Self = if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
        Self::X86_64_WINDOWS
    } else if cfg!(all(target_arch = "x86_64", not(target_os = "windows"))) {
        Self::X86_64_LINUX
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        Self::AARCH64_MACOS
    } else if cfg!(all(target_arch = "aarch64", not(target_os = "macos"))) {
        Self::AARCH64_LINUX
    } else {
        Self::X86_32_LINUX
    };

    pub const AARCH64_LINUX: Self = Self {
        long_double: (16, 16),
        ..Self::X86_64_LINUX
    };
    pub const AARCH64_MACOS: Self = Self {
        long_double: (8, 8),
        ..Self::X86_64_LINUX
    };

    pub const X86_32_LINUX: Self = Self {
        long: (4, 4),
        long_long: (8, 4),
        double: (8, 4),
        long_double: (12, 4),
        ptr: (4, 4),
        ..Self::X86_64_LINUX
    };

    pub const X86_64_WINDOWS: Self = Self {
        long: (4, 4),
        long_long: (8, 8),
        ..Self::X86_64_LINUX
    };
    pub const X86_64_LINUX: Self = Self {
        void: (0, 1),
        bool: (1, 1),
        char: (1, 1),
        short: (2, 2),
        int: (4, 4),
        long: (8, 8),
        long_long: (8, 8),
        float: (4, 4),
        double: (8, 8),
        long_double: (16, 16),
        ptr: (8, 8),
    };
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldLayout(pub String, pub usize, pub CType);

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub info: TypeInfo,
    pub fields: Vec<FieldLayout>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutCtx<'a> {
    decls: &'a CDecls,
    target: Target,
    tag_layouts: HashMap<usize, Layout>,
    visiting: HashSet<usize>,
}

impl<'a> LayoutCtx<'a> {
    pub fn with_target(decls: &'a CDecls, target: Target) -> LayoutCtx<'a> {
        Self {
            decls,
            target,
            visiting: HashSet::new(),
            tag_layouts: HashMap::new(),
        }
    }
    pub fn new(decls: &'a CDecls) -> LayoutCtx<'a> {
        Self::with_target(decls, Target::HOST)
    }

    pub fn get_typedef_info(&mut self, name: &str) -> Option<TypeInfo> {
        self.type_info(
            self.decls
                .typedefs
                .iter()
                .find_map(|i| (i.0 == name).then_some(&i.1))?,
        )
    }
    pub fn get_tag_layout(&mut self, name: &str) -> Option<Layout> {
        self.tag_layouts
            .get(self.decls.tag_mapper.get(name)?)
            .cloned()
    }

    #[inline]
    pub fn union_info(&mut self, cu: &CUnion) -> Option<Layout> {
        if cu.1.is_empty() {
            return None;
        }

        let mut size = 0usize;
        let mut align = 1usize;
        let mut fields = Vec::with_capacity(cu.1.len());
        for (name, ty) in cu.1.iter() {
            let (s, a) = self.type_info(ty)?;
            size = size.max(s);
            align = align.max(a);
            fields.push(FieldLayout(name.clone(), 0, ty.clone()));
        }
        Some(Layout {
            info: (align_up(size, align), align),
            fields,
        })
    }
    #[inline]
    pub fn struct_info(&mut self, cs: &CStruct) -> Option<Layout> {
        if cs.1.is_empty() {
            return None;
        }

        let mut offset = 0usize;
        let mut align = 1usize;
        let mut fields = Vec::with_capacity(cs.1.len());

        for (name, ty) in &cs.1 {
            let (s, a) = self.type_info(ty)?;
            offset = align_up(offset, a);
            fields.push(FieldLayout(name.clone(), offset, ty.clone()));
            offset += s;
            align = align.max(a);
        }

        Some(Layout {
            info: (align_up(offset, align), align),
            fields,
        })
    }

    #[inline]
    pub fn type_info(&mut self, ty: &CType) -> Option<TypeInfo> {
        Some(match ty {
            CType::Base(b) => base_info(b, &self.target),
            CType::Ptr(_) => self.target.ptr,
            CType::Array(c, Some(len)) => {
                let (s, a) = self.type_info(&*c)?;
                (s * *len, a)
            }
            CType::Struct(s) => return self.struct_info(s).map(|l| l.info),
            CType::Enum(e) => return enum_info(e, &self.target),
            CType::Union(u) => return self.union_info(u).map(|l| l.info),
            CType::Function(_) => return None,
            CType::TagRef(r) => {
                if let Some(layout) = self.tag_layouts.get(r) {
                    layout.info
                } else {
                    if !self.visiting.insert(*r) {
                        return None;
                    }

                    let c = self.decls.get_tag_by_id(*r)?;
                    let layout = match c {
                        CTag::Enum(e) => Layout {
                            info: enum_info(e, &self.target)?,
                            fields: vec![],
                        },
                        CTag::Struct(s) => self.struct_info(s)?,
                        CTag::Union(u) => self.union_info(u)?,
                    };
                    self.tag_layouts.insert(*r, layout.clone());
                    self.visiting.remove(r);
                    layout.info
                }
            }
            CType::TypeRef(r) => self.decls.get_type(*r).and_then(|ty| self.type_info(ty))?,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        cdef::CTag,
        layout::LayoutCtx,
        parser::{Parser, tokenize},
    };

    #[test]
    fn test_layout() {
        let mut parser = Parser::new(
            tokenize(
                r#"
struct Inner {
    char  a;
    long  b;
};

union U {
    char  arr[5];
    int   i;
};

struct Complex {
    char          c;
    int           i;
    struct Inner  inner;
    char          arr[5];
    short         s;
    union U       u;
    char          tail;
};
"#,
            )
            .unwrap(),
        );
        parser.ffis().expect("");

        let CTag::Struct(s) = parser.decls.get_tag("Complex").unwrap() else {
            unreachable!()
        };
        println!("{:?}", LayoutCtx::new(&parser.decls).struct_info(s));
    }
}
