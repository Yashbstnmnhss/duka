#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Descriptor {
    // immediate
    INil,
    ITrue,
    IFalse,
    IInt,

    // target[usize]
    Constant(usize),
    Local(usize),
    Temp(usize),
    Upvalue(usize),

    // table-related
    Index(usize, usize),
    IndexField(usize, usize),
    IndexInt(usize, u8),
    IndexUpField(usize, usize), // this also include global variable

    // function-related
    Call(usize),
    Closure(usize),
    Function(usize),
    VarArgs,

    // arithmetic
    Unary(usize),
    Binary(usize, usize),

    // control flow
    Test(Box<Descriptor>, Vec<usize>, Vec<usize>),
    Compare(usize, usize, Vec<usize>, Vec<usize>),
}
