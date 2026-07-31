use duka_macros::instructions;

/*
 * Let's think about this stuff.
 * So, to implement a thing called WAM(Warren's abstract machine),
 *     what should I do? I'm not very clear about that.
 * Whatever, I tried to define a part of instructions here, though I haven't figured out them
 */

instructions! {
    mode {
        Empty(),
        N(N[8]),
        NC(N[8], C[8]),
        AN(A[address], N[8]),
        V(A[address]),
        VV(A[address], B[address]),
        VC(A[address], B[address]),
        VVV(A[address], B[address], C[address])
    }
    flags()
    impl[8] {
        TRY[AN](),

        UnifyConst[NC](),
        UnifyVar[V](),
        UnifyVarVar[VV](),
        UnifyVarConst[VC](),

        BindVar[VV](),
        BindConst[VC](),

        Cons[VVV](),
        Head[VV](),
        Tail[VV](),
        EmptyList[V](),

        Fail[Empty](),
        Succeed[Empty](),
        Cut[V](),

        Call[N](),
        Proceed[Empty]()
    }
    as LogicInstruction
}
