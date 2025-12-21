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
}

pub mod cvm {
    const_str!(STACK = "stack");
    const_str!(CONST = "constants");
    const_str!(UPVAL = "upvalues");
}

pub mod ctype {
    const_str!(NUM = "number");
    const_str!(FLO = "float");
    const_str!(INT = "int");
    const_str!(STR = "string");
    const_str!(TAB = "table");
    const_str!(FUN = "function");

    const_str!(CMP = "comparable");
    const_str!(PRO = "prototype");
}

pub mod sugar {
    const_str!(priv builtin TYPE_IS_TABLE = "タイプ_イズ_テーブル");
    const_str!(sugar LINQ_TABLE = "リスト");
    const_str!(sugar LINQ_INDEX = "インダクス");
}

const_str!(GLOBAL = "_ENV");

/// ### Meta method name list for duka meta table
#[derive(Debug, Info)]
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
