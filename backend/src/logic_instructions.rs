use duka_macros::instructions;

/*
 * Let's think about this stuff.
 * So, to implement a thing called WAM(Warren's abstract machine),
 *     what should I do? I'm not very clear about that.
 * Whatever, I tried to define a part of instructions here, though I haven't figure out them
 */

instructions! {
    mode {
        Empty(),
        AN(A[address], N[8]),
        V(A[address]),
        VV(A[address], B[address]),
        VC(A[address], B[address]),
        VVV(A[address], B[address], C[address])
    }
    flags()
    impl[8] {
        TRY[AN](),

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
        Cut[V]()
    }
    as LogicInstruction
}
