use duka_macros::duka_builtin_type_def;
use duka_shared::dtype::{CmpOp, NumPred, Pattern, Pred, TextPred, Type};
use duka_shared::value::ConstValue;

use super::{TypeResult, plain, string_arg};
use crate::analyzer::tyval::{TypeValue, TypeValueExt};

fn pat_of(base: Type, pred: Pred) -> TypeResult {
    plain(tv!(Type::Pattern(Box::new(Pattern { base, pred }))))
}

fn num_pat(v: Box<[TypeValue]>, op: CmpOp) -> TypeResult {
    let mut v = v.into_iter();
    let base = v.next().expect("Checked").to_type();
    if !base.base().is_numeric() {
        return Err(format!("this pattern needs a numeric type, got {base}"));
    }
    let value = match v.next().expect("Checked").to_type() {
        Type::Literal(cv) => cv,
        other => return Err(format!("expected a number, got {other}")),
    };
    pat_of(base, Pred::Num(NumPred::Cmp(op, value)))
}

fn text_pat(v: Box<[TypeValue]>, f: fn(Box<str>) -> TextPred) -> TypeResult {
    let mut v = v.into_iter();
    let base = v.next().expect("Checked").to_type();
    if base.base() != &Type::String {
        return Err(format!("this pattern needs a string type, got {base}"));
    }
    let part = string_arg(&v.next().expect("Checked"))?;
    pat_of(base, Pred::Text(f(part.into())))
}

fn len_pat(v: Box<[TypeValue]>, op: CmpOp) -> TypeResult {
    let mut v = v.into_iter();
    let base = v.next().expect("Checked").to_type();
    if base.base() != &Type::String {
        return Err(format!("this pattern needs a string type, got {base}"));
    }
    let n = match v.next().expect("Checked").to_type() {
        Type::Literal(ConstValue::Int(n)) => n,
        other => return Err(format!("expected a length, got {other}")),
    };
    pat_of(base, Pred::Text(TextPred::Len(op, n)))
}

fn combine_pat(v: Box<[TypeValue]>, and: bool) -> TypeResult {
    let mut v = v.into_iter();
    let left = v.next().expect("Checked").to_type();
    let right = v.next().expect("Checked").to_type();
    let (Type::Pattern(a), Type::Pattern(b)) = (&left, &right) else {
        return Err(format!("expected two patterns, got {left} and {right}"));
    };
    let pred = match (&a.pred, &b.pred) {
        (Pred::Num(x), Pred::Num(y)) => Pred::Num(if and {
            NumPred::And(vec![x.clone(), y.clone()].into())
        } else {
            NumPred::Or(vec![x.clone(), y.clone()].into())
        }),
        (Pred::Text(x), Pred::Text(y)) => Pred::Text(if and {
            TextPred::And(vec![x.clone(), y.clone()].into())
        } else {
            TextPred::Or(vec![x.clone(), y.clone()].into())
        }),
        _ => return Err("a number pattern and a text pattern cannot combine".to_owned()),
    };
    pat_of(a.base.clone(), pred)
}

fn not_pat(v: Box<[TypeValue]>) -> TypeResult {
    let inner = v.into_iter().next().expect("Checked").to_type();
    let Type::Pattern(p) = &inner else {
        return Err(format!("expected a pattern, got {inner}"));
    };
    let pred = match &p.pred {
        Pred::Num(x) => Pred::Num(NumPred::Not(Box::new(x.clone()))),
        Pred::Text(x) => Pred::Text(TextPred::Not(Box::new(x.clone()))),
    };
    pat_of(p.base.clone(), pred)
}

duka_builtin_type_def! {
    type mod patterns
    name "Patterns"
    doc "Pattern constructors"

    fn {
        #[duka_builtin(doc = "Numbers in range [a, b)", count = 2)]
        fn RangePat(v: Box<[TypeValue]>) -> TypeResult {
            let mut v = v.into_iter();
            let base = v.next().expect("Checked").to_type();
            if !base.base().is_numeric() {
                return Err(format!("this pattern needs a numeric type, got {base}"));
            }
            let a = match v.next().expect("Checked").to_type() {
                Type::Literal(cv) => cv,
                other => return Err(format!("expected a number, got {other}")),
            };
            let b = match v.next().expect("Checked").to_type() {
                Type::Literal(cv) => cv,
                other => return Err(format!("expected a number, got {other}")),
            };
            pat_of(base, Pred::Num(NumPred::And([
                NumPred::Cmp(CmpOp::Le, a),
                NumPred::Cmp(CmpOp::Gt, b)
            ].into())))
        },

        #[duka_builtin(doc = "Numbers greater than N", count = 2)]
        fn GtPat(v: Box<[TypeValue]>) -> TypeResult { num_pat(v, CmpOp::Gt) },

        #[duka_builtin(doc = "Numbers greater than or equal to N", count = 2)]
        fn GePat(v: Box<[TypeValue]>) -> TypeResult { num_pat(v, CmpOp::Ge) },

        #[duka_builtin(doc = "Numbers less than N", count = 2)]
        fn LtPat(v: Box<[TypeValue]>) -> TypeResult { num_pat(v, CmpOp::Lt) },

        #[duka_builtin(doc = "Numbers less than or equal to N", count = 2)]
        fn LePat(v: Box<[TypeValue]>) -> TypeResult { num_pat(v, CmpOp::Le) },

        #[duka_builtin(doc = "Numbers equal to N", count = 2)]
        fn EqPat(v: Box<[TypeValue]>) -> TypeResult { num_pat(v, CmpOp::Eq) },

        #[duka_builtin(doc = "Numbers other than N", count = 2)]
        fn NePat(v: Box<[TypeValue]>) -> TypeResult { num_pat(v, CmpOp::Ne) },

        #[duka_builtin(doc = "Strings containing PART", count = 2)]
        fn HasPat(v: Box<[TypeValue]>) -> TypeResult { text_pat(v, TextPred::Has) },

        #[duka_builtin(doc = "Strings starting with PART", count = 2)]
        fn StartsPat(v: Box<[TypeValue]>) -> TypeResult { text_pat(v, TextPred::Starts) },

        #[duka_builtin(doc = "Strings ending with PART", count = 2)]
        fn EndsPat(v: Box<[TypeValue]>) -> TypeResult { text_pat(v, TextPred::Ends) },

        #[duka_builtin(doc = "Strings matching REGEX in full", count = 2)]
        fn RegexPat(v: Box<[TypeValue]>) -> TypeResult { text_pat(v, TextPred::Regex) },

        #[duka_builtin(doc = "Strings at least N characters long", count = 2)]
        fn LenGePat(v: Box<[TypeValue]>) -> TypeResult { len_pat(v, CmpOp::Ge) },

        #[duka_builtin(doc = "Strings at most N characters long", count = 2)]
        fn LenLePat(v: Box<[TypeValue]>) -> TypeResult { len_pat(v, CmpOp::Le) },

        #[duka_builtin(doc = "Strings longer than N characters", count = 2)]
        fn LenGtPat(v: Box<[TypeValue]>) -> TypeResult { len_pat(v, CmpOp::Gt) },

        #[duka_builtin(doc = "Strings shorter than N characters", count = 2)]
        fn LenLtPat(v: Box<[TypeValue]>) -> TypeResult { len_pat(v, CmpOp::Lt) },

        #[duka_builtin(doc = "Strings exactly N characters long", count = 2)]
        fn LenEqPat(v: Box<[TypeValue]>) -> TypeResult { len_pat(v, CmpOp::Eq) },

        #[duka_builtin(doc = "Both patterns hold", count = 2)]
        fn AndPat(v: Box<[TypeValue]>) -> TypeResult { combine_pat(v, true) },

        #[duka_builtin(doc = "Either pattern holds", count = 2)]
        fn OrPat(v: Box<[TypeValue]>) -> TypeResult { combine_pat(v, false) },

        #[duka_builtin(doc = "The pattern does not hold", count = 1)]
        fn NotPat(v: Box<[TypeValue]>) -> TypeResult { not_pat(v) },
    }
}
