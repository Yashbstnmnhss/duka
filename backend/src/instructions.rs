use std::{borrow::Borrow, fmt::Display};

use duka_macros::instructions;

fn rk(v: impl Display, k: impl Borrow<bool>) -> String {
    format!("{}[{}]", if *k.borrow() { "K" } else { "R" }, v)
}
fn range(
    target: impl Display,
    from: impl Borrow<Address>,
    count: impl Borrow<u32>,
    var: bool,
) -> String {
    let from = *from.borrow() as u32;
    let count = *count.borrow();
    if var && count == 1 || count == 0 {
        return String::new();
    }
    format!(
        "{target}[{from}{}]",
        match count {
            0 if var => "..".to_owned(),
            n => format!("..{}", from + n - 1),
        }
    )
}

// 这就是DSL
instructions! {
    /*
        A, B, C...: Address for register
            x: Extra argument
            k: With constant marker
        Im: Immediate operand
        Is: Immediate signed operand
        Ka: Constant number & index
        Kb: Constant boolean
        Sj: Signed jumping (offset)
        Sn: Signed number
    */
    mode {
        ABC(A[address], B[address], C[address]),
        ABCk(A[address], B[address], C[address], Kb[bool]),

        KbAIm(Kb[bool], A[address], Im[16]),
        ABk(A[address], B[address], Kb[bool]),
        AKa(A[address], Ka[17]),
        ASn(A[address], Sn[17 signed]),
        Ax(Ax[25]),
        A(A[address]),
        Ak(A[address], Kb[bool]),
        AB(A[address], B[address]),
        Sj(Sj[25 signed]),
        Empty(),
    }

    flags(setA, test, inTop, outTop, metaMethod, extra)

    /*
        Suffix:
            K: 参数其一为常量池中的索引
            I: immediate 立即值
            X: 下一条是ExtraArg命令
    */
    impl[7] {
        Move[AB](setA) -> |a, b| format!("R[{a}] = R[{b}]"),
        LoadI[ASn](setA) -> |a, n| format!("R[{a}] = {n}"),
        LoadK[AKa](setA) -> |a, i| format!("R[{a}] = K[{i}]"),
        LoadKX[A](setA, extra) -> |a| format!("R[{a}] = Extra"),
        LoadFalse[A](setA) -> |a| format!("R[{a}] = false"),
        LoadFalseSkip[A](setA) -> |a| format!("R[{a}] = false; pc++"),
        LoadTrue[A](setA) -> |a| format!("R[{a}] = true"),
        LoadNil[AKa](setA) -> |a, b| format!("{} = nil", range("R", a, b, false)),
        GetUpVal[AKa](setA) -> |a, b| format!("R[{a}] = UpVal[{b}]"),
        SetUpVal[AKa]() -> |a, b| format!("UpVal[{b}] = R[{a}]"),

        GetTabUp[ABC](setA) -> |a, b, c| format!("R[{a}] = UpVal[{b}][K[{c}]]"),
        GetTable[ABC](setA) -> |a, b, c| format!("R[{a}] = R[{b}][R[{c}]]"),
        GetI[ABC](setA) -> |a, b, c| format!("R[{a}] = R[{b}][{c}]"),
        GetField[ABC](setA) -> |a, b, c| format!("R[{a}] = R[{b}][K[{c}]]"),

        SetTabUp[ABCk]() -> |a, b, c, k: &bool| format!("UpVal[{a}][K[{b}]] = {}", rk(c, *k)),
        SetTable[ABC](),//
        SetI[ABC](),//
        SetField[ABC](),//

        NewTable[ABC](setA, extra),//

        Self_[ABC](setA) -> |a, b, c| format!(""),// R[A+1] = R[B]; R[A] = R[B][RC(C):string]

        AddI[ABC](setA) -> |a, b, im| format!(""),// + immediate number

        AddK[ABC](setA),//
        SubK[ABC](setA),//
        MulK[ABC](setA),//
        ModK[ABC](setA),//
        PowK[ABC](setA),//
        DivK[ABC](setA),//
        IDivK[ABC](setA),//

        BitAndK[ABC](setA),// &
        BitOrK[ABC](setA),// |
        BitXorK[ABC](setA),// ~

        ShiftRI[ABC](setA),// >> immediate number
        ShiftLI[ABC](setA),// << immediate number

        Add[ABC](setA),// +
        Sub[ABC](setA),// -
        Mul[ABC](setA),// *
        Mod[ABC](setA),// %
        Pow[ABC](setA),// ^
        Div[ABC](setA),// /
        IDiv[ABC](setA),// //

        BitAnd[ABC](setA),// and
        BitOr[ABC](setA),// or
        BitXor[ABC](setA),// xor
        ShiftL[ABC](setA),// >>
        ShiftR[ABC](setA),// <<

        MMBinary[ABC](metaMethod),// call meta method
        MMBinaryI[ASn](metaMethod),// call meta method with immediate
        MMBinaryK[ABC](metaMethod),// call meta method with constant

        Minus[AB](setA) -> |a, b| format!("R[{a}] = -R[{b}]"),// -
        BitNot[AB](setA) -> |a, b| format!("R[{a}] = ~R[{b}]"),// ~
        Not[AB](setA) -> |a, b| format!("R[{a}] = not R[{b}]"),// not
        Length[AB](setA) -> |a, b| format!("R[{a}] = len(R[{b}])"),// #

        Concat[AKa](setA) -> |a, ct| format!("R[{a}] = concat({})", range("R", a, ct, false)),// ..

        Close[A](),//
        MarkToBeClosed[A](),//
        Jump[Sj]() -> |o: &i32| format!("pc {} {}", o.is_negative().then_some("-=").unwrap_or("+="), o.abs()),//
        Equal[AB](test) ,// ==
        Less[AB](test),// <
        LessEqual[AB](test),// <=

        EqualK[AB](test),// == const
        EqualI[KbAIm](test),// == immediate
        LessI[KbAIm](test),// < immediate
        LessEqualI[KbAIm](test),// <= immediate
        GreaterI[KbAIm](test),// > immediate
        GreaterEqualI[KbAIm](test),// >= immediate

        Test[Ak](test) -> |a, k| format!("if {} == true then pc++", rk(a, k)),//
        TestSet[ABk](test, setA),//

        Call[ABC](inTop, outTop, setA) -> |a, b, c: &u8| format!("call {a}({arg}) -> {r}", arg = range("R", a + 1, (b - 1) as u32, false), r = range("R", 0, *c as u32, true)),//
        CallSet[ABC](inTop, outTop, setA), //
        TailCall[AB](inTop, outTop, setA),//

        Return[AKa](inTop) -> |a, ct| format!("return {}", range("R", a, ct, true)),// return R[A] ... R[A + B - 2]
        Return0[Empty]() -> || "return".to_owned(),

        Yield[ABC](inTop, outTop) -> |from, count: &u8, wanted| format!("yield {r} -> {wanted}", r = range("R", from, *count as u32, true)), // yield a coroutine
        Go[ABC](outTop) -> |id, from, count: &u8| format!("go coroutine#{id}({})", range("R", from, *count as u32, true)), // do a coroutine call

        ForPrepare[AKa](setA),//
        ForLoop[AKa](setA),//

        // generic for loop
        TForPrepare[AKa](),
        TForCall[AB](),
        TForLoop[AKa](setA),

        SetList[ABC](inTop),

        Closure[AKa](setA) -> |a, i| format!("R[{a}] = Closures[{i}]"),

        VarArgPrepare[Ax](inTop, setA) -> |c| format!("VarArg = {}", range("R", 0, c, false)),
        VarArg[AKa](outTop, setA) -> |a, ct| format!("{} = VarArg", range("R", a, ct, false)),

        ExtraArg[Ax]() -> |e| format!("Extra = {e}") // 给**下一条**指令扩展参数(位数多)
    }
    as Instruction
}
