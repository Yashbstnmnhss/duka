use duka_macros::instructions;

// 这就是DSL
instructions! {
    mode {
        ABC(A[8], k[bool], B[8], C[8]),
        ABX(A[8], Bx[17]),
        AsBx(A[8], sBx[17 signed]),
        Ax(Ax[25]),
        SJ(sJ[25 signed])
    }

    flags(setA, test, useTop, setTop, metaMethod)

    impl[7] {
        GetGlobal[ABX](setA),
        LoadConst[ABX](),
        Call[ABX](),
        LoadI[AsBx](),
        LoadK[AsBx](),
        Test[SJ](test),
    }
    as Instruction
}
