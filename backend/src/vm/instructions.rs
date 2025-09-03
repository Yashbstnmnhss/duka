use duka_macros::instructions;

// 这就是DSL
instructions! {
    mode {
        ABC(A[address], B[address], C[address]),
        ABCk(A[address], B[address], C[address], k[bool]),
        ABsC(A[address], B[address], sC[9 signed]),
        ABk(A[address], B[address], k[bool]),
        AsBk(A[address], sB[16 signed], k[bool]),
        ABx(A[address], Bx[17]),
        AsBx(A[address], sBx[17 signed]),
        Ax(Ax[25]),
        A(A[address]),
        Ak(A[address], k[bool]),
        AB(A[address], B[address]),
        SJ(sJ[25 signed]),
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
        Move[AB](setA), // R[A] = R[B]
        LoadI[AsBx](setA), // R[A] = sBx
        LoadK[ABx](setA), // 常量 R[A] = K[Bx]
        LoadKX[A](setA, extra), // extra arg
        LoadFalse[A](setA), // R[A] = false
        LoadFalseSkip[A](setA), // R[A] = false; pc++
        LoadTrue[A](setA),// R[A] = true
        LoadNil[ABx](setA),// R[A], ..., R[A+B] = nil
        GetUpVal[AB](setA),// R[A] = UpVal[B]
        SetUpVal[AB](),// UpVal[B] = R[A]

        GetTabUp[ABC](setA),//
        GetTable[ABC](setA),//
        GetI[ABC](setA),//
        GetField[ABC](setA),//

        SetTabUp[ABC](),//
        SetTable[ABC](),//
        SetI[ABC](),//
        SetField[ABC](),//

        NewTable[ABC](setA, extra),//

        Self_[ABC](setA),// R[A+1] = R[B]; R[A] = R[B][RC(C):string]

        AddI[ABC](setA),// + immediate number

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
        MMBinaryI[AsBx](metaMethod),// call meta method with immediate
        MMBinaryK[ABC](metaMethod),// call meta method with constant

        Minus[AB](setA),// -
        BitNot[AB](setA),// ~
        Not[AB](setA),// not
        Length[AB](setA),// #

        Concat[ABx](setA),// ..

        Close[A](),//
        MarkToBeClosed[A](),//
        Jump[SJ](),//
        Equal[AB](test),// ==
        Less[AB](test),// <
        LessEqual[AB](test),// <=

        EqualK[AB](test),// == const
        EqualI[AB](test),// == immediate
        LessI[AB](test),// < immediate
        LessEqualI[AB](test),// <= immediate
        GreaterI[AB](test),// > immediate
        GreaterEqualI[AB](test),// >= immediate

        Test[Ak](test),//
        TestSet[ABk](test, setA),//

        Call[ABC](inTop, outTop, setA),//
        CallSet[ABC](inTop, outTop, setA), //
        TailCall[ABC](inTop, outTop, setA),//

        Return[ABx](inTop),// return R[A] ... R[A + B - 2]
        Return0[Empty](),// return
        // Return1[A](),// return R[A] why?

        ForPrepare[ABx](setA),//
        ForLoop[ABx](setA),//

        // generic for loop
        TForPrepare[ABx](),
        TForCall[AB](),
        TForLoop[ABx](setA),

        SetList[ABC](inTop),

        Closure[ABx](setA),

        VarArgPrepare[Ax](inTop, setA),
        VarArg[ABx](outTop, setA),

        ExtraArg[Ax]() // 给下一条指令扩展参数(位数多)
    }
    as Instruction
}
