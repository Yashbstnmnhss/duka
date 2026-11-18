use duka_macros::duka_builtin_type_def;
use duka_shared::dtype::{CmpOp, NumPred, Pattern, Pred, TextPred, Type};
use duka_shared::value::{ConstValue, DukaFloat, DukaFloatWrapper, DukaInt};

use super::{TypeResult, plain, string_arg};
use crate::analyzer::tyval::{TypeValue, TypeValueExt};

fn pat_of(base: Type, pred: Pred) -> TypeResult {
    plain(tv!(Type::Pattern(Box::new(Pattern { base, pred }))))
}

/// 数值谓词的界跟着底座走:整型底座取精确 `i64`(浮点字面量须是整值),
/// 浮点底座把整数字面量加宽成 `f64`。
enum Bound {
    Int(DukaInt),
    Float(DukaFloat),
}

fn bound_of(value: &ConstValue, base: &Type) -> Result<Bound, String> {
    if base.base() == &Type::Int {
        match value {
            ConstValue::Int(i) => Ok(Bound::Int(*i)),
            ConstValue::Float(f)
                if f.fract() == 0.0 && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 =>
            {
                Ok(Bound::Int(*f as i64))
            }
            other => Err(format!(
                "an integer pattern needs an integer bound, got {other}"
            )),
        }
    } else {
        match value {
            ConstValue::Int(i) => Ok(Bound::Float(*i as DukaFloat)),
            ConstValue::Float(f) => Ok(Bound::Float(*f)),
            other => Err(format!("expected a number, got {other}")),
        }
    }
}

fn num_pred(op: CmpOp, value: &ConstValue, base: &Type) -> Result<Pred, String> {
    Ok(match bound_of(value, base)? {
        Bound::Int(n) => Pred::Int(NumPred::cmp(op, n)),
        Bound::Float(n) => Pred::Num(NumPred::cmp(op, DukaFloatWrapper::from(n))),
    })
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
    let pred = num_pred(op, &value, &base)?;
    pat_of(base, pred)
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
    pat_of(base, Pred::Text(TextPred::Len(NumPred::cmp(op, n))))
}

fn combine_num<N: PartialOrd + Copy>(x: &NumPred<N>, y: &NumPred<N>, and: bool) -> NumPred<N> {
    if and {
        x.clone().and(y.clone())
    } else {
        x.clone().or(y.clone())
    }
}

fn combine_pat(v: Box<[TypeValue]>, and: bool) -> TypeResult {
    let mut v = v.into_iter();
    let left = v.next().expect("Checked").to_type();
    let right = v.next().expect("Checked").to_type();
    let (Type::Pattern(a), Type::Pattern(b)) = (&left, &right) else {
        return Err(format!("expected two patterns, got {left} and {right}"));
    };
    let pred = match (&a.pred, &b.pred) {
        (Pred::Int(x), Pred::Int(y)) => Pred::Int(combine_num(x, y, and)),
        (Pred::Num(x), Pred::Num(y)) => Pred::Num(combine_num(x, y, and)),
        (Pred::Text(x), Pred::Text(y)) => Pred::Text(if and {
            TextPred::And(vec![x.clone(), y.clone()].into())
        } else {
            TextPred::Or(vec![x.clone(), y.clone()].into())
        }),
        _ => return Err("patterns must combine over the same base type".to_owned()),
    };
    pat_of(a.base.clone(), pred)
}

fn not_pat(v: Box<[TypeValue]>) -> TypeResult {
    let inner = v.into_iter().next().expect("Checked").to_type();
    let Type::Pattern(p) = &inner else {
        return Err(format!("expected a pattern, got {inner}"));
    };
    let pred = match &p.pred {
        Pred::Int(x) => Pred::Int(x.clone().not()),
        Pred::Num(x) => Pred::Num(x.clone().not()),
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
            let pred = match (bound_of(&a, &base)?, bound_of(&b, &base)?) {
                (Bound::Int(a), Bound::Int(b)) => Pred::Int(NumPred::half_open(a, b)),
                (Bound::Float(a), Bound::Float(b)) => Pred::Num(NumPred::half_open(
                    DukaFloatWrapper::from(a),
                    DukaFloatWrapper::from(b),
                )),
                _ => unreachable!("both bounds follow one base type"),
            };
            pat_of(base, pred)
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
