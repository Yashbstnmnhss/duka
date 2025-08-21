use duka_macros::instructions;

// 这就是DSL
instructions! {
    mode {
        ABC(A[address], B[address], C[address]),
        ABsC(A[address], B[address], sC[9 signed]),
        ABk(A[address], B[address], k[bool]),
        AsBk(A[address], sB[16 signed], k[bool]),
        ABx(A[address], Bx[17]),
        AsBx(A[address], sBx[17 signed]),
        Ax(Ax[25]),
        A(A[address]),
        AB(A[address], B[address]),
        SJ(sJ[25 signed]),
        Empty(),
    }

    flags(setA, test, inTop, outTop, metaMethod)

    /*
        Suffix:
            Const: 参数其一为常量池中的索引
            I: immediate 立即值
            X: 为上一条指令扩展参数
    */
    impl[7] {
        Move[AB](setA), // R[A] = R[B]
        LoadI[AsBx](setA), // R[A] = sBx
        LoadConst[ABx](setA), // 常量 R[A] = Const[Bx]
        LoadConstX[A](setA), // extra arg
        LoadFalse[A](setA), // R[A] = false
        LoadFalseSkip[A](setA), // R[A] = false; pc++
        LoadTrue[A](setA),// R[A] = true
        LoadNil[A](setA),// R[A], ..., R[A+B] = nil
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

        NewTable[ABC](setA),//

        Self_[ABC](setA),// R[A+1] = R[B]; R[A] = R[B][RC(C):string]

        AddI[ABC](setA),//

        AddConst[ABC](setA),//
        SubConst[ABC](setA),//
        MulConst[ABC](setA),//
        ModConst[ABC](setA),//
        PowConst[ABC](setA),//
        DivConst[ABC](setA),//
        IDivConst[ABC](setA),//

        BitAndConst[ABC](setA),//
        BitOrConst[ABC](setA),//
        BitXorConst[ABC](setA),//

        ShiftRI[ABC](setA),//
        ShiftLI[ABC](setA),//

        Add[ABC](setA),//
        Sub[ABC](setA),//
        Mul[ABC](setA),//
        Mod[ABC](setA),//
        Pow[ABC](setA),//
        Div[ABC](setA),//
        IDiv[ABC](setA),//

        BitAnd[ABC](setA),//
        BitOr[ABC](setA),//
        BitXor[ABC](setA),//
        ShiftL[ABC](setA),//
        ShiftR[ABC](setA),//

        MMBin[ABC](metaMethod),//
        MMBinI[AsBx](metaMethod),//
        MMBinConst[ABC](metaMethod),//

        Minus[AB](setA),//
        BitNot[AB](setA),//
        Not[AB](setA),//
        Length[AB](setA),//

        Concat[AB](setA),//

        Close[A](),//
        MarkToBeClosed[A](),//
        Jump[SJ](),//
        Equal[AB](test),//
        Less[AB](test),//
        LessEqual[AB](test),//

        EqualConst[AB](test),//
        EqualI[AB](test),//
        LessI[AB](test),//
        LessEqualI[AB](test),//
        GreaterI[AB](test),//
        GreaterEqual[AB](test),//

        Test[A](test),//
        TestSet[A](test, setA),//

        Call[ABC](inTop, outTop, setA),//
        TailCall[ABC](inTop, outTop, setA),//

        Return[ABC](inTop),//
        Return0[Empty](),//
        Return1[A](),//

        ForLoop[ABx](setA),//
        ForPrepare[ABx](setA),//

        TForPrepare[ABx](),
        TForCall[AB](),
        TForLoop[ABx](setA),

        SetList[ABC](inTop),

        Closure[ABx](setA),

        VarArg[AB](outTop, setA),
        VarArgPrepare[A](inTop, setA),

        ExtraArg[Ax]()
    }
    as Instruction
}
