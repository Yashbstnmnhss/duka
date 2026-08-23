use duka_shared::builtin::Builtins;
use duka_shared::builtin::GlobalBuiltins;
use duka_shared::dtype::Type;
use duka_shared::utils::OrError;
use duka_shared::value::ConstValue;
use std::sync::LazyLock;
use std::sync::RwLock;

use crate::analyzer::tyval::TypeValue;

pub type TypeBuiltinFunc = fn(Box<[Type]>) -> Result<TypeValue, String>;

pub static TYPE_BUILTINS: GlobalBuiltins<TypeBuiltinFunc> = LazyLock::new(|| {
    RwLock::new(
        Builtins::<TypeBuiltinFunc>::new()
            .register("IsSubType", |v| {
                (v.len() != 2).then_error(|| "Expected two type argument".to_owned())?;
                Ok(TypeValue::Type(Type::Any))
            })
            .register("HasVarArg", |v| {
                if v.len() == 0 {
                    return Err("Expected one type argument".to_owned());
                }
                Ok(TypeValue::Type(if let Type::Function(Some(ft)) = &v[0] {
                    Type::Literal(ConstValue::Bool(ft.var_arg))
                } else {
                    Type::Literal(ConstValue::Bool(false))
                }))
            })
            .register("HasRetVarArg", |v| {
                if v.len() == 0 {
                    return Err("Expected one type argument".to_owned());
                }
                Ok(TypeValue::Type(if let Type::Function(Some(ft)) = &v[0] {
                    Type::Literal(ConstValue::Bool(ft.return_var_arg))
                } else {
                    Type::Literal(ConstValue::Bool(false))
                }))
            }),
    )
});
