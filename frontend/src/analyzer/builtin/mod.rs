use duka_shared::builtin::Builtins;
use duka_shared::builtin::GlobalBuiltins;
use duka_shared::docs::{MetaInfo, MetaItemInfo};
use duka_shared::dtype::Type;
use duka_shared::utils::OrError;
use duka_shared::value::ConstValue;
use std::sync::LazyLock;
use std::sync::RwLock;

use crate::analyzer::tyval::TypeValue;

macro_rules! type_functions {
    ($(
        #[duka_builtin(doc = $doc: literal)]
        $name: literal($p: ident [$count: literal]) $body: block
    ),*) => {
        pub static TYPE_BUILTINS: GlobalBuiltins<TypeBuiltinFunc> = LazyLock::new(|| {
            RwLock::new(
                Builtins::<TypeBuiltinFunc>::new()
                    $(.register(
                        $name,
                        |$p| {
                            ($p.len() < $count).then_error(|| concat!("Expected ", stringify!($count)," type argument").to_owned())?;
                            $body
                        }
                    ))*
            )
        });
        pub const TYPE_BUILTINS_META: MetaInfo = MetaInfo {
            name: "type-context",
            doc: "Builtins for type-context",
            example: None,
            info: MetaItemInfo::Module {
                inner: &[
                    $(MetaInfo {
                        name: $name,
                        doc: $doc,
                        example: None,
                        info: MetaItemInfo::TypeFunction {
                            param_count: $count
                        },
                        flags: &[],
                    }),*
                ]
            },
            flags: &[("feature", &["type-context"])],
        };
    };
}

pub type TypeBuiltinFunc = fn(Box<[TypeValue]>) -> Result<TypeValue, String>;

fn get_str(bytes: &[u8]) -> Result<&str, String> {
    str::from_utf8(bytes).map_err(|e| e.to_string())
}

type_functions! {
    #[duka_builtin(doc = "Throw an error with a message")]
    "Error"(v[1]) {
        Err(v[0].to_string())
    },
    #[duka_builtin(doc = "If A isn't true, throws an error with message B")]
    "Assert"(v[2]) {
        let ty = v[0].to_type();
        matches!(ty, Type::Literal(ConstValue::Bool(true)))
            .then_some(TypeValue::Type(ty))
            .ok_or(v[1].to_string())
    },
    #[duka_builtin(doc = "Stringify a type")]
    "Stringify"(v[1]) {
        Ok(TypeValue::Type(
            Type::Literal(ConstValue::String(
                v[0].to_string().into_bytes().into_boxed_slice()
            ))
        ))
    },
    #[duka_builtin(doc = "Pack types from type array into a union type")]
    "Union"(v[1]) {
        let t = v[0].to_type();
        Ok(TypeValue::Type(
            if let Type::TypeTuple(types) = t {
                Type::Union(types.into_boxed_slice())
            }
            else {
               return Ok(v.into_iter().next().expect("Checked"))
            }
        ))
    },
    #[duka_builtin(doc = "Unpack a union type")]
    "Unpack"(v[1]) {
        let t = v[0].to_type();
        Ok(TypeValue::Type(
            if let Type::Union(types) = t {
                Type::TypeTuple(types.into_vec())
            }
            else {
                return Ok(v.into_iter().next().expect("Checked"))
            }
        ))
    },
    #[duka_builtin(doc = "Whether B is a sub type of A")]
    "IsSubType"(v[2]) {
        Ok(TypeValue::Type(Type::Literal(ConstValue::Bool(
            v[1].accepts(&v[0]),
        ))))
    },
    #[duka_builtin(doc = "Whether A is in B")]
    "In"(v[2]) {
        if let Some(other) = &v[0].as_type() {
            if let Some(Type::Union(types)) = &v[1].as_type()  {
                return Ok(Type::Literal(ConstValue::Bool(types.contains(other))).into())
            }
            else if let Some(Type::TypeTuple(types)) = &v[1].as_type() {
                return Ok(Type::Literal(ConstValue::Bool(types.contains(other))).into())
            }
            else if let Some(Type::TypeTable(types)) = &v[1].as_type() &&
                let Type::Literal(c) = other
            {
                return Ok(Type::Literal(ConstValue::Bool(types.iter().any(|i| &i.0 == c))).into())
            }
        }
        Ok(Type::Literal(ConstValue::Bool(false)).into())
    },
    #[duka_builtin(doc = "Whether the function type has var-arg parameter")]
    "HasVarArg"(v[1]) {
        Ok(TypeValue::Type(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
            Type::Literal(ConstValue::Bool(ft.var_arg))
        } else {
            Type::Literal(ConstValue::Bool(false))
        }))
    },
    #[duka_builtin(doc = "Whether the function type has var-arg returns")]
    "HasRetVarArg"(v[1]) {
        Ok(TypeValue::Type(if let Some(Type::Function(Some(ft))) = &v[0].as_type() {
            Type::Literal(ConstValue::Bool(ft.return_var_arg))
        } else {
            Type::Literal(ConstValue::Bool(false))
        }))
    },
    #[duka_builtin(doc = "Remove B in union or type array A")]
    "Exclude"(v[2]) {
        let mut v = v.into_iter();
        let first = v.next().expect("Checked");
        let second = v.next().expect("Checked");
        if let Some(t) = second.as_type() {
            match first {
                TypeValue::Type(Type::Union(types)) if types.contains(t) => {
                    return Ok(Type::Union(types.into_iter().filter(|i| i == t).collect()).into())
                },
                TypeValue::Type(Type::TypeTuple(types)) if types.contains(t) => {
                    return Ok(Type::TypeTuple(types.into_iter().filter(|i| i == t).collect()).into())
                },
                _ => ()
            }
        }
        Ok(first)
    },
    #[duka_builtin(doc = "Return true if literal string type A ends with B")]
    "EndsWith"(v[2]) {
        Ok(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(end))) = &v[1].as_type() {
                let tar = get_str(tar)?;
                let end = get_str(end)?;
                Type::Literal(ConstValue::Bool(tar.ends_with(end)))
            } else {
                Type::Literal(ConstValue::Bool(false))
            },
        ))
    },
    #[duka_builtin(doc = "Return true if literal string type A starts with B")]
    "StartsWith"(v[2]) {
        Ok(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(start))) = &v[1].as_type() {
                let tar = get_str(tar)?;
                let start = get_str(start)?;
                Type::Literal(ConstValue::Bool(tar.starts_with(start)))
            } else {
                Type::Literal(ConstValue::Bool(false))
            },
        ))
    },
    #[duka_builtin(doc = "Slice string literal type A with start index B and optional end index C")]
    "Slice"(v[2]) {
        Ok(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::Int(start))) = &v[1].as_type() {
                let start = *start as usize;
                let tar = get_str(tar)?;
                Type::Literal(ConstValue::String(if let Some(TypeValue::Type(Type::Literal(ConstValue::Int(end)))) = v.get(2) {
                    let end = *end as usize;
                    &tar[start..end]
                } else {
                    &tar[start..]
                }.as_bytes().into()))
            } else {
                return Ok(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Split string literal type A by separator string literal type B")]
    "Split"(v[2]) {
        Ok(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(tar))) = &v[0].as_type() && let Some(Type::Literal(ConstValue::String(sep))) = &v[1].as_type() {
                let tar = get_str(tar)?;
                let sep = get_str(sep)?;
                Type::TypeTuple(
                    tar.split(sep).map(|i| Type::Literal(ConstValue::String(i.as_bytes().into()))).collect()
                )
            } else {
                return Ok(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Convert a string literal type into uppercase")]
    "Uppercase"(v[1]) {
        Ok(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                Type::Literal(ConstValue::String(
                    str.iter().map(|b| b.to_ascii_uppercase()).collect(),
                ))
            } else {
                return Ok(v.into_iter().next().expect("Checked"))
            },
        ))
    },
    #[duka_builtin(doc = "Convert a string literal type into lowercase")]
    "Lowercase"(v[1]) {
        Ok(TypeValue::Type(
            if let Some(Type::Literal(ConstValue::String(str))) = &v[0].as_type() {
                Type::Literal(ConstValue::String(
                    str.iter().map(|b| b.to_ascii_lowercase()).collect(),
                ))
            } else {
                return Ok(v.into_iter().next().expect("Checked"))
            },
        ))
    }
}
