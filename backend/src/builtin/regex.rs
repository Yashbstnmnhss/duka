use crate::{
    builtin::arg::oks,
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeDukaTable, RuntimeValue},
};
use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def, duka_user_data};
use duka_shared::regex::{Compiled, Runner, compile, escape, find_all};
use duka_shared::value::DukaInt;

duka_builtin_def! {
    mod regex
    doc "RegEx for duka"
    fn {
        meta:
            impl_search,
            impl_find_all,
            impl_compile,
            impl_escape,
            impl_replace,
            impl_replace_all,
            impl_is_match
    }
    const {}
    userdata {
        meta: CompiledRegex
    }
}

duka_user_data! {
    #[duka_builtin(doc = "Compiled regex object")]
    struct CompiledRegex {
        inner: Compiled
    }
    constructor fn new(pattern: &str) -> Result<Self, DukaRuntimeError> {
        Ok(Self {
            inner: compile(pattern).map_err(|e| DukaRuntimeError::Custom(e.to_string()))?
        })
    }
    #[duka_builtin(name = "replace_all", params(self: userdata, text: string, replacement: string, from: int = 0), returns(string))]
    fn impl_replace_all(&self, heap: &mut Heap, text: String, replacement: String, from: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
        let r = Runner::new(&self.inner).replace(&text, &replacement, from as usize, usize::MAX).map_err(|e| DukaRuntimeError::Custom(e.to_string()))?;
        Ok(RuntimeValue::from_string(heap, r))
    },
    #[duka_builtin(name = "replacen", params(self: userdata, text: string, replacement: string, from: int = 0, times: int = 1), returns(string))]
    fn impl_replacen(&self, heap: &mut Heap, text: String, replacement: String, from: DukaInt, times: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
        let r = Runner::new(&self.inner).replace(&text, &replacement, from as usize, times as usize).map_err(|e| DukaRuntimeError::Custom(e.to_string()))?;
        Ok(RuntimeValue::from_string(heap, r))
    },
    #[duka_builtin(name = "replace", params(self: userdata, text: string, replacement: string, from: int = 0), returns(string))]
    fn impl_replace(&self, heap: &mut Heap, text: String, replacement: String, from: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
        let r = Runner::new(&self.inner).replace(&text, &replacement, from as usize, 1).map_err(|e| DukaRuntimeError::Custom(e.to_string()))?;
        Ok(RuntimeValue::from_string(heap, r))
    },
    #[duka_builtin(name = "is_match", params(self: userdata, text: string, from: int = 0), returns(bool))]
    fn impl_is_match(&self, text: String, from: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
        Ok(RuntimeValue::Bool(Runner::new(&self.inner).search(&text, from as usize).is_none()))
    },
    #[duka_builtin(name = "search", params(self: userdata, text: string, from: int = 0), returns(bool, vararg))]
    fn impl_search(&self, heap: &mut Heap, text: String, from: DukaInt) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        let Some(m) = Runner::new(&self.inner).search(&text, from as usize) else {
            return Ok(vec![RuntimeValue::Bool(false)]);
        };
        let items = m
            .captures
            .into_iter()
            .map(|(from, to)| RuntimeValue::from_string(heap, text[from..to].to_string()))
            .collect();
        let nameds = m
            .named_captures
            .into_iter()
            .map(|(key, (from, to))| (
                RuntimeValue::from_string(heap, key.into_string()),
                RuntimeValue::from_string(heap, text[from..to].to_string())
            ))
            .collect();

        Ok(oks([RuntimeValue::Array(
            heap.alloc(GcCell::new(RuntimeDukaArray { items })),
        ), RuntimeValue::Table(
            heap.alloc(GcCell::new(RuntimeDukaTable {
                metatable: None,
                inner: nameds
            }))
        )]))
    },
    #[duka_builtin(name = "find_all", params(self: userdata, text: string), returns(array, array))]
    fn impl_find_all(&self, heap: &mut Heap, text: String) -> Result<(RuntimeValue, RuntimeValue), DukaRuntimeError> {
        let mut arrays = vec![];
        let mut arrays2 = vec![];
        for m in find_all(&self.inner, &text) {
            let items = m
                .captures
                .into_iter()
                .map(|(from, to)| RuntimeValue::from_string(heap, text[from..to].to_string()))
                .collect();
            let nameds = m
                .named_captures
                .into_iter()
                .map(|(key, (from, to))| (
                    RuntimeValue::from_string(heap, key.into_string()),
                    RuntimeValue::from_string(heap, text[from..to].to_string())
                ))
                .collect();
            arrays.push(RuntimeValue::Array(heap.alloc(GcCell::new(RuntimeDukaArray { items }))));
            arrays2.push(RuntimeValue::Table(
                heap.alloc(GcCell::new(RuntimeDukaTable {
                    metatable: None,
                    inner: nameds
                }))
            ));
        }
        Ok((RuntimeValue::Array(
            heap.alloc(GcCell::new(RuntimeDukaArray { items: arrays })),
        ),RuntimeValue::Array(
            heap.alloc(GcCell::new(RuntimeDukaArray { items: arrays2 })),
        )))
    }
}

#[duka_builtin(doc = "Escape a regex string", params(pattern: string), returns(string))]
fn impl_escape(heap: &mut Heap, pattern: String) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::from_string(heap, escape(&pattern)))
}

#[duka_builtin(doc = "Compile a pattern into CompiledRegex", params(pattern: string), returns(any))]
fn impl_compile(heap: &mut Heap, pattern: String) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(CompiledRegex::new(&pattern)?.into_value(heap))
}

#[duka_builtin(
    name = "replace_all", 
    doc = "Replace given string by given pattern in text to replacement (replace **all**)",
    params(pattern: string, text: string, pattern: string, from: int = 0),
    returns(string)
)]
fn impl_replace_all(
    heap: &mut Heap,
    text: String,
    pattern: String,
    replacement: String,
    from: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    CompiledRegex::new(&pattern)?.impl_replace_all(heap, text, replacement, from)
}
#[duka_builtin(
    name = "replace", 
    doc = "Replace given string by given pattern in text to replacement (replace **once** by default)",
    params(pattern: string, text: string, pattern: string, from: int = 0, times: int = 1),
    returns(string)
)]
fn impl_replace(
    heap: &mut Heap,
    text: String,
    pattern: String,
    replacement: String,
    from: DukaInt,
    times: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    CompiledRegex::new(&pattern)?.impl_replacen(heap, text, replacement, from, times)
}
#[duka_builtin(
    name = "search", 
    doc = "Search a substring by given pattern in text (search once)",
    params(pattern: string, text: string, from: int = 0),
    returns(vararg)
)]
fn impl_search(
    heap: &mut Heap,
    pattern: String,
    text: String,
    from: DukaInt,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    CompiledRegex::new(&pattern)?.impl_search(heap, text, from)
}
#[duka_builtin(
    name = "find_all", 
    doc = "Find all strings by given pattern (global mode)",
    params(pattern: string, text: string),
    returns(array, array),
    return_doc = "Nested array, `[[captures1...], [captures2...]]`"
)]
fn impl_find_all(
    heap: &mut Heap,
    pattern: String,
    text: String,
) -> Result<(RuntimeValue, RuntimeValue), DukaRuntimeError> {
    CompiledRegex::new(&pattern)?.impl_find_all(heap, text)
}

#[duka_builtin(
    name = "is_match", 
    doc = "Whether given text matches given pattern",
    params(pattern: string, text: string, from: int = 0),
    returns(bool),
)]
fn impl_is_match(
    pattern: String,
    text: String,
    from: DukaInt,
) -> Result<RuntimeValue, DukaRuntimeError> {
    CompiledRegex::new(&pattern)?.impl_is_match(text, from)
}
