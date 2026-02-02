use std::{borrow::Borrow, fmt::Display};

use duka_macros::instructions;

#[inline(always)]
fn rk(v: impl Display, k: impl Borrow<bool>) -> String {
    format!("{}[{}]", if *k.borrow() { "K" } else { "R" }, v)
}
#[inline(always)]
fn rng(
    target: impl Display,
    from: impl Borrow<Address>,
    count: impl Borrow<u32>,
    var: bool,
    empty: impl Display,
) -> String {
    let from = *from.borrow() as u32;
    let count = *count.borrow();
    if count == 0 || var && count == 1 {
        return empty.to_string();
    }
    format!(
        "{target}[{from}{}]",
        match count {
            0 if var => "..".to_owned(),
            n => format!("..{}", from + n - 1),
        }
    )
}
#[inline(always)]
fn rng_empty(
    target: impl Display,
    from: impl Borrow<Address>,
    count: impl Borrow<u32>,
    var: bool,
) -> String {
    rng(target, from, count, var, "")
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
        N:  Unsigned number
        M:  Metamethod ID
    */
    mode {
        ABC(A[address], B[address], C[address]),
        ABKb(A[address], B[address], Kb[bool]),
        ABCk(A[address], B[address], C[address], Kb[bool]),
        ANCk(A[address], N[8], C[address], Kb[bool]),
        ABN(A[address], B[address], N[9]),
        ABSn(A[address], B[address], Sn[9 signed]),
        KbAIm(Kb[bool], A[address], Im[16]),
        AKaKb(A[address], Ka[16], Kb[bool]),
        ABK(A[address], B[address], K[9]),
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
        LoadNil[AKa](setA) -> |a, b| format!("{} = nil", rng_empty("R", a, b, false)),
        GetUpVal[AKa](setA) -> |a, b| format!("R[{a}] = UpVal[{b}]"),
        SetUpVal[AKa]() -> |a, b| format!("UpVal[{b}] = R[{a}]"),

        /*
            xxxTable: R(b)
            xxxField: K(b)
         */

        GetTabUp[ABK](setA) -> |a, b, k| format!("R[{a}] = UpVal[{b}][K[{k}]]"),
        GetTable[ABC](setA) -> |a, b, c| format!("R[{a}] = R[{b}][R[{c}]]"),
        GetI[ABN](setA) -> |a, b, c| format!("R[{a}] = R[{b}][{c}]"),
        GetField[ABK](setA) -> |a, b, c| format!("R[{a}] = R[{b}][K[{c}]]"),

        SetTabUp[ANCk]() -> |a, b, c, k: &bool| format!("UpVal[{a}][K[{b}]] = {}", rk(c, *k)),
        SetTable[ABCk](),//
        SetI[ANCk](),//
        SetField[ANCk](),//

        NewTable[AKaKb](setA) -> |to, n, new: &bool| if !new {format!("R[{to}] = {{}} (len={n})")} else {format!("R[{to}] = {{ from K[{n}] }}")},//

        Self_[ABCk](setA) -> |a, b, c, k: &bool| format!("R[{}] = R[{}]; R[{}] = R[{}][{}:string]", a + 1, b, a, b, rk(c, *k)),

        AddI[ABSn](setA) -> |a, b, im| format!("R[{a}] = R[{b}] + {im}"),// + immediate number

        AddK[ABC](setA) -> |a, b, k| format!("R[{a}] = R[{b}] + K[{k}]:number"),//
        SubK[ABC](setA) -> |a, b, k| format!("R[{a}] = R[{b}] - K[{k}]:number"),//
        MulK[ABC](setA) -> |a, b, k| format!("R[{a}] = R[{b}] * K[{k}]:number"),//
        ModK[ABC](setA) -> |a, b, k| format!("R[{a}] = R[{b}] % K[{k}]:number"),//
        PowK[ABC](setA) -> |a, b, k| format!("R[{a}] = R[{b}] ^ K[{k}]:number"),//
        DivK[ABC](setA) -> |a, b, k| format!("R[{a}] = R[{b}] / K[{k}]:number"),//
        IDivK[ABC](setA) -> |a, b, k| format!("R[{a}] = R[{b}] // K[{k}]:number"),//

        BitAndK[ABC](setA),// &
        BitOrK[ABC](setA),// |
        BitXorK[ABC](setA),// ~

        ShiftRI[ABSn](setA),// >> immediate number
        // NO NEED ShiftLI[ABC](setA),// << immediate number

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
        MMBinaryI[ABSn](metaMethod),// call meta method with immediate
        MMBinaryK[ABC](metaMethod),// call meta method with constant

        Minus[AB](setA) -> |a, b| format!("R[{a}] = -R[{b}]"),// -
        BitNot[AB](setA) -> |a, b| format!("R[{a}] = ~R[{b}]"),// ~
        Not[AB](setA) -> |a, b| format!("R[{a}] = not R[{b}]"),// not
        Length[AB](setA) -> |a, b| format!("R[{a}] = len(R[{b}])"),// #

        Concat[AKa](setA) -> |a, ct| format!("R[{a}] = concat({})", rng_empty("R", a, ct, false)),// ..

        Close[A](),//
        MarkToBeClosed[A](),//
        Jump[Sj]() -> |o: &i32| format!("pc {} {}", o.is_negative().then_some("-=").unwrap_or("+="), o.abs()),//
        Equal[AB](test) ,// ==
        Less[AB](test),// <
        LessEqual[AB](test),// <=

        EqualK[ABKb](test),// == const
        EqualI[KbAIm](test),// == immediate
        LessI[KbAIm](test),// < immediate
        LessEqualI[KbAIm](test),// <= immediate
        GreaterI[KbAIm](test),// > immediate
        GreaterEqualI[KbAIm](test),// >= immediate

        Test[Ak](test) -> |a, k| format!("if {} == true then pc++", rk(a, k)),//
        TestSet[ABKb](test, setA),//

        Call[ABC](inTop, outTop, setA) -> |a, b, c| format!("call R[{a}]({arg}) -> [{c}]", arg = rng_empty("R", a + 1, (b - 1) as u32, false)),//
        SysCall[ABC](inTop, outTop, setA) -> |a, b, c| format!("syscall @{a}({arg}) -> [{c}]", arg = rng_empty("R", a + 1, (b - 1) as u32, false)),
        CallSet[ABC](inTop, outTop, setA), //
        TailCall[AB](inTop, outTop, setA)-> |a, b| format!("return call R[{a}]({arg})", arg = rng_empty("R", a + 1, (b - 1) as u32, false)),//

        Return[AKa](inTop) -> |a, count| format!("return {}", rng_empty("R", a, count, true)),// return R[A] ... R[A + B - 2]
        Return0[Empty]() -> || "return".to_owned(),

        Yield[ABC](inTop, outTop) -> |from, count: &u8, wanted| format!("yield {r} -> [{wanted}]", r = rng_empty("R", from, *count as u32, true)), // yield a coroutine
        Go[ABC](outTop) -> |id, from, count: &u8| format!("go coroutine#{id}({})", rng_empty("R", from, *count as u32, true)), // do a coroutine call

        ForPrepare[AKa](setA) -> |a, ka| format!("<prepare counters> for ... do else pc += {ka}"),//
        ForLoop[AKa](setA) -> |a, ka| format!("if continue then pc -= {ka}"),//

        // generic for loop
        TForPrepare[AKa](),
        TForCall[AB](),
        TForLoop[AKa](setA),

        SetList[ABC](inTop),

        Closure[AKa](setA) -> |a, i| format!("R[{a}] = Closures[{i}]"),

        VarArgPrepare[Ax](inTop, setA) -> |c| format!("VarArg = {}", rng("R", 0, c, false, "nil")),
        VarArg[AKa](outTop, setA) -> |a, ct| format!("{} = VarArg", rng_empty("R", a, ct, false)),

        ExtraArg[Ax]() -> |e| format!("Extra = {e}") // 给**下一条**指令扩展参数(位数多)
    }
    as Instruction
}
