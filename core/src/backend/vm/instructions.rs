use duka_macros::instructions;

// 这就是DSL
instructions! {
    mode {
        ABC(A[address], k[bool], B[address], C[address]),
        ABx(A[address], Bx[17]),
        AsBx(A[address], sBx[17 signed]),
        Ax(Ax[25]),
        A(A[address]),
        AB(A[address], B[address]),
        SJ(sJ[25 signed]),
        Empty(),
    }

    flags(setA, test, useTop, setTop, metaMethod)

    impl[7] {
        Move[AB](setA),
        LoadI[AsBx](setA),
        LoadK[AsBx](setA),
        LoadFalse[A](setA),
        LoadTrue[A](setA),
        LoadFalseSkip[A](setA),
        LoadNil[A](setA),

        GetGlobal[ABx](setA),
        LoadConst[ABx](),

        NewTable[ABC](),
        SetField[ABC](),
        SetTable[ABx](),
        SetList[ABx](),

        Call[ABx](),
        Test[SJ](test),

        ExtraArg[Ax](),
        Return0[Empty](),
        Return[ABC](),
    }
    as Instruction
}
