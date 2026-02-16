use std::{borrow::Borrow, fmt::Display};

use duka_macros::instructions;
use duka_shared::constants::MetaMethod;

#[inline(always)]
fn rk(v: impl Display, k: impl Borrow<bool>) -> String {
    format!("{}[{}]", if *k.borrow() { "K" } else { "R" }, v)
}
#[inline]
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
        ABK(A[address], B[address], K[9]),
        AKa(A[address], Ka[17]),
        ASn(A[address], Sn[17 signed]),

        ABM(A[address], B[address], M[enum MetaMethod[8]]),
        AKMKb(A[address], K[address], M[enum MetaMethod[8]], Kb[bool]),
        ASnMKb(A[address], Sn[8 signed], M[enum MetaMethod[8]], Kb[bool]),

        Ax(Ax[25]),
        A(A[address]),
        Ak(A[address], Kb[bool]),
        AB(A[address], B[address]),
        Sj(Sj[25 signed]),
        Empty(),
    }

    flags(set_a, test, in_top, out_top, meta_method, extra)

    /*
        Suffix:
            K: 参数其一为常量池中的索引
            I: immediate 立即值
            X: 下一条是ExtraArg命令
    */
    impl[7] {
        Move[AB](set_a) -> |a, b| format!("R[{a}] = R[{b}]"),
        LoadI[ASn](set_a) -> |a, n| format!("R[{a}] = {n}"),
        LoadK[AKa](set_a) -> |a, i| format!("R[{a}] = K[{i}]"),
        LoadKX[A](set_a, extra) -> |a| format!("R[{a}] = Extra"),
        LoadFalse[A](set_a) -> |a| format!("R[{a}] = false"),
        LoadTrue[A](set_a) -> |a| format!("R[{a}] = true"),
        LoadNil[AKa](set_a) -> |a, b| format!("{} = nil", rng_empty("R", a, b, false)),
        GetUpVal[AKa](set_a) -> |a, b| format!("R[{a}] = UpVal[{b}]"),
        SetUpVal[AKa]() -> |a, b| format!("UpVal[{b}] = R[{a}]"),

        /*
            xxxTable: R(b)
            xxxField: K(b)
         */

        GetTabUp[ABK](set_a) -> |a, b, k| format!("R[{a}] = UpVal[{b}][K[{k}]]"),
        GetTable[ABC](set_a) -> |a, b, c| format!("R[{a}] = R[{b}][R[{c}]]"),
        GetI[ABN](set_a) -> |a, b, c| format!("R[{a}] = R[{b}][{c}]"),
        GetField[ABK](set_a) -> |a, b, c| format!("R[{a}] = R[{b}][K[{c}]]"),

        SetTabUp[ANCk]() -> |a, b, c, k: &bool| format!("UpVal[{a}][K[{b}]] = {}", rk(c, *k)),
        SetTable[ABCk](),//
        SetI[ANCk](),//
        SetField[ANCk](),//

        NewTable[A](set_a) -> |to| format!("R[{to}] = {{}}"),//

        Self_[ABCk](set_a) -> |a, b, c, k: &bool| format!("R[{}] = R[{}]; R[{}] = R[{}][{}:string]", a + 1, b, a, b, rk(c, *k)),

        AddI[ABSn](set_a) -> |a, b, im| format!("R[{a}] = R[{b}] + {im}"),// + immediate number

        AddK[ABC](set_a) -> |a, b, k| format!("R[{a}] = R[{b}] + K[{k}]:number"),//
        SubK[ABC](set_a) -> |a, b, k| format!("R[{a}] = R[{b}] - K[{k}]:number"),//
        MulK[ABC](set_a) -> |a, b, k| format!("R[{a}] = R[{b}] * K[{k}]:number"),//
        ModK[ABC](set_a) -> |a, b, k| format!("R[{a}] = R[{b}] % K[{k}]:number"),//
        PowK[ABC](set_a) -> |a, b, k| format!("R[{a}] = R[{b}] ^ K[{k}]:number"),//
        DivK[ABC](set_a) -> |a, b, k| format!("R[{a}] = R[{b}] / K[{k}]:number"),//
        IDivK[ABC](set_a) -> |a, b, k| format!("R[{a}] = R[{b}] // K[{k}]:number"),//

        BitAndK[ABC](set_a),// &
        BitOrK[ABC](set_a),// |
        BitXorK[ABC](set_a),// ~

        ShiftRI[ABSn](set_a),// >> immediate number
        // NO NEED ShiftLI[ABC](set_a),// << immediate number

        Add[ABC](set_a),// +
        Sub[ABC](set_a),// -
        Mul[ABC](set_a),// *
        Mod[ABC](set_a),// %
        Pow[ABC](set_a),// ^
        Div[ABC](set_a),// /
        IDiv[ABC](set_a),// //
        Xor[ABC](set_a), // xor

        BitAnd[ABC](set_a),// and
        BitOr[ABC](set_a),// or
        BitXor[ABC](set_a),// xor
        ShiftL[ABC](set_a),// >>
        ShiftR[ABC](set_a),// <<

        MMBinary[ABM](meta_method),// call meta method
        MMBinaryI[ASnMKb](meta_method),// call meta method with immediate
        MMBinaryK[AKMKb](meta_method),// call meta method with constant

        Minus[AB](set_a) -> |a, b| format!("R[{a}] = -R[{b}]"),// -
        BitNot[AB](set_a) -> |a, b| format!("R[{a}] = ~R[{b}]"),// ~
        Not[AB](set_a) -> |a, b| format!("R[{a}] = not R[{b}]"),// not
        Length[AB](set_a) -> |a, b| format!("R[{a}] = len(R[{b}])"),// #

        Concat[AKa](set_a) -> |a, ct| format!("R[{a}] = concat({})", rng_empty("R", a, ct, false)),// ..

        Close[A](),//
        MarkToBeClosed[A](),//
        Jump[Sj]() -> |o: &i32| format!("pc {} {}", if o.is_negative() { "-=" } else { "+=" }, o.abs()),//
        Equal[AB](test) ,// ==
        Less[AB](test),// <
        LessEqual[AB](test),// <=

        EqualK[ABKb](test),// == const
        EqualI[KbAIm](test),// == immediate
        LessI[KbAIm](test),// < immediate
        LessEqualI[KbAIm](test),// <= immediate
        GreaterI[KbAIm](test),// > immediate
        GreaterEqualI[KbAIm](test),// >= immediate

        Test[Ak](test) -> |a, k| format!("if R[{a}] == {k} then pc++"),//

        Call[ABC](in_top, out_top, set_a) -> |a, b, c| format!("call R[{a}]({arg}) -> [{c}]", arg = rng_empty("R", a + 1, (b - 1) as u32, false)),//
        SysCall[ABC](in_top, out_top, set_a) -> |a, b, c| format!("syscall @{a}({arg}) -> [{c}]", arg = rng_empty("R", a + 1, (b - 1) as u32, false)),
        TailCall[ABC](in_top, out_top, set_a)-> |a, b, c| format!("tailcall R[{a}]({arg})", arg = rng_empty("R", a + 1, (b - 1) as u32, false)),//

        Return[AKa](in_top) -> |a, count| format!("return {}", rng_empty("R", a, count, true)),// return R[A] ... R[A + B - 2]
        Return0[Empty]() -> || "return".to_owned(),

        Yield[ABC](in_top, out_top) -> |from, count: &u8, wanted| format!("yield {r} -> [{wanted}]", r = rng_empty("R", from, *count as u32, true)), // yield a coroutine
        Go[ABC](in_top, out_top, set_a) -> |id, from, count: &u8| format!("go coroutine#{id}({})", rng_empty("R", from, *count as u32, true)), // do a coroutine call
        Spawn[AB](out_top, set_a) -> |to, func| format!("spawn R[{to}] <- R[{func}]"),

        ForPrepare[AKa](set_a) -> |a, ka| format!("<prepare counters> for ... do else pc += {ka}"),//
        ForLoop[AKa](set_a) -> |a, ka| format!("if continue then pc -= {ka}"),//

        // generic for loop
        TForPrepare[AKa](),
        TForCall[AB](),
        TForLoop[AKa](set_a),

        SetList[ABC](in_top),

        Closure[AKa](set_a) -> |a, i| format!("R[{a}] = Closures[{i}]"),

        VarArgPrepare[Ax](in_top, set_a) -> |c| format!("VarArg = {}", rng("R", 0, c, false, "nil")),
        VarArg[AKa](out_top, set_a) -> |a, ct| format!("{} = VarArg", rng_empty("R", a, ct, false)),

        ExtraArg[Ax]() -> |e| format!("Extra = {e}") // 给**下一条**指令扩展参数(位数多)
    }
    as Instruction
}
