use duka_macros::instructions;

// 这就是DSL
instructions! {
    /*
        A, B, C...: Address for register
            x: Extra argument
        Im: Immediate operand
        Is: Immediate signed operand
        Ka: Constant number & index
        Kb: Constant boolean
        Sj: Signed jumping (offset)
        Sn: Signed number
    */
    mode {
        ABC(A[address], B[address], C[address]),
        KbAIm(Kb[bool], A[address], Im[16]),
        ABKb(A[address], B[address], Kb[bool]),
        ABx(A[address], Bx[17]),
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
        Move[AB](setA), // R[A] = R[B]
        LoadI[ASn](setA), // R[A] = sBx
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
        MMBinaryI[ASn](metaMethod),// call meta method with immediate
        MMBinaryK[ABC](metaMethod),// call meta method with constant

        Minus[AB](setA),// -
        BitNot[AB](setA),// ~
        Not[AB](setA),// not
        Length[AB](setA),// #

        Concat[ABx](setA),// ..

        Close[A](),//
        MarkToBeClosed[A](),//
        Jump[Sj](),//
        Equal[AB](test),// ==
        Less[AB](test),// <
        LessEqual[AB](test),// <=

        EqualK[AB](test),// == const
        EqualI[KbAIm](test),// == immediate
        LessI[KbAIm](test),// < immediate
        LessEqualI[KbAIm](test),// <= immediate
        GreaterI[KbAIm](test),// > immediate
        GreaterEqualI[KbAIm](test),// >= immediate

        Test[Ak](test),//
        TestSet[ABKb](test, setA),//

        Call[ABC](inTop, outTop, setA),//
        CallSet[ABC](inTop, outTop, setA), //
        TailCall[ABC](inTop, outTop, setA),//

        Return[ABx](inTop),// return R[A] ... R[A + B - 2]
        Return0[Empty](),// return

        // Yield[Empty](inTop), // yield a coroutine
        // Coroutine[Empty](), // do a coroutine call

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

        ExtraArg[Ax]() // 给**下一条**指令扩展参数(位数多)
    }
    as Instruction
}
