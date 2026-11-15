use crate::analyzer::TypeValue;
use duka_shared::dtype::Constraint;

pub type TypeResult = Result<(TypeValue, Vec<Constraint>), String>;

#[inline(always)]
pub fn plain(value: TypeValue) -> TypeResult {
    Ok((value, vec![]))
}
