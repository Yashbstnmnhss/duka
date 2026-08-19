use duka_shared::builtin::Builtins;
use duka_shared::builtin::GlobalBuiltins;
use duka_shared::dtype::Type;
use duka_shared::value::ConstValue;
use std::sync::LazyLock;
use std::sync::RwLock;

pub type TypeBuiltinFunc = fn(Box<[Type]>) -> Result<Type, String>;

pub static TYPE_BUILTINS: GlobalBuiltins<TypeBuiltinFunc> = LazyLock::new(|| {
    RwLock::new(
        Builtins::<TypeBuiltinFunc>::new()
            .register("IsSubType", |v| todo!())
            .register("HasVarArg", |v| {
                if v.len() == 0 {
                    return Err("Expected one type argument".to_owned());
                }
                Ok(if let Type::Function(Some(ft)) = &v[0] {
                    Type::Literal(ConstValue::Bool(ft.var_arg))
                } else {
                    Type::Literal(ConstValue::Bool(false))
                })
            })
            .register("HasRetVarArg", |v| {
                if v.len() == 0 {
                    return Err("Expected one type argument".to_owned());
                }
                Ok(if let Type::Function(Some(ft)) = &v[0] {
                    Type::Literal(ConstValue::Bool(ft.return_var_arg))
                } else {
                    Type::Literal(ConstValue::Bool(false))
                })
            }),
    )
});
