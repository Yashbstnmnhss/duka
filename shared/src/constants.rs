use duka_macros::Info;

macro_rules! const_str {
    ($n: ident = $c: literal) => {
        pub const $n: &'static str = $c;
    };
    (sugar $n: ident = $c: literal) => {
        pub const $n: &'static str = concat!("_s_", $c);
    };
    (priv builtin $n: ident = $c: literal) => {
        pub const $n: &'static str = concat!("_b_", $c);
    };
    ([$name:ident; $count: literal] $($n: ident = $c: literal),*) => {
        $(const_str!($n = $c);)*
        pub const $name: [&'static str; $count] = [$($n),*];
    };
}

const_str!(SOURCE_SUFFIX = ".duka");
const_str!(COMPILED_SUFFIX = ".dukac");

pub mod cvm {
    const_str!(STACK = "stack");
    const_str!(CONST = "constants");
    const_str!(UPVAL = "up_values");
}

pub mod catt {
    const_str!(CLOSE = "close");
    const_str!(CONST = "const");
    const_str!(INLINE = "inline");
}

pub mod cpar {
    const_str!(EXP = "<exp>");
    const_str!(VAR = "<var>");
    const_str!(CAL = "<call>");
    const_str!(INT = "<integer>");
    //im sorry for this, but I really don't know how to deal it gracefully
    const_str!(SRY = "<*>");
    const_str!(DISCARD = "_");
}

pub mod clex {
    const_str!(NAMEOF = "nameof");
    const_str!(STRINGIFY = "stringify");
    const_str!(CONCAT = "concat");
    const_str!(COUNTER = "counter");
    const_str!(WHEN = "when");
    const_str!(NONEMPTY = "nonempty");
    const_str!(LENIS = "lenis");
    const_str!(ID = "<identifier>");
}

pub mod ctype {
    const_str!(NUM = "number");
    const_str!(FLO = "float");
    const_str!(INT = "int");
    const_str!(STR = "string");
    const_str!(TAB = "table");
    const_str!(FUN = "function");
    const_str!(BOO = "bool");

    const_str!(NIL = "nil");
    const_str!(CMP = "comparable");
    const_str!(PRO = "prototype");
    const_str!(CLO = "closure");
}

pub mod csugar {
    const_str!(priv builtin TYPE_IS_TABLE = "タイプ_イズ_テーブル");
    const_str!(sugar LINQ_TABLE = "リスト");
    const_str!(sugar LINQ_INDEX = "インダクス");
}

pub mod ccallish {
    const_str!(
        [CALLISHES; 3]
        SPAWN = "spawn",
        GO = "go",
        YIELD = "yield"
    );
}

pub mod cgen {
    pub const MAX_REGISTER_COUNT: usize = 256;
    pub const MAX_LOCAL_COUNT: usize = 200;

    pub const ENV_UPVAL_IDX: usize = 0;
    const_str!(MAIN = "main");
    const_str!(GLOBAL = "_ENV");
    const_str!(SELF = "self");
}
pub const MAX_EXPANDING_DEPTH: u16 = 256;

/// ### Meta method name list for duka meta table
/// NOTICE: NAME OF THEM MUST BE SHORTER THAN <code>SHORT_STR_LEN</code>
///
#[derive(Debug, Info, PartialEq, Clone)]
#[idcard(u8)]
pub enum MetaMethod {
    #[name("__index")]
    Index,
    #[name("__newindex")]
    NewIndex,
    #[name("__gc")]
    Gc,
    #[name("__mode")]
    Mode,
    #[name("__len")]
    Len,
    #[name("__eq")]
    Eq,
    #[name("__add")]
    Add,
    #[name("__sub")]
    Sub,
    #[name("__mul")]
    Mul,
    #[name("__mod")]
    Mod,
    #[name("__pow")]
    Pow,
    #[name("__div")]
    Div,
    #[name("__idiv")]
    IDiv,
    #[name("__band")]
    BAnd,
    #[name("__bor")]
    BOr,
    #[name("__bxor")]
    BXor,
    #[name("__shl")]
    ShL,
    #[name("__shr")]
    ShR,
    #[name("__unm")]
    Unm,
    #[name("__bnot")]
    BNot,
    #[name("__lt")]
    LT,
    #[name("__le")]
    LE,
    #[name("__concat")]
    Concat,
    #[name("__call")]
    Call,
    #[name("__close")]
    Close,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum MetaMethodAction {
    #[default]
    Default,
    Swap,
    Inverse,
}

impl TryFrom<u8> for MetaMethod {
    type Error = u8;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::from_disc(value)
    }
}
impl From<MetaMethod> for u8 {
    fn from(value: MetaMethod) -> Self {
        value.disc()
    }
}
