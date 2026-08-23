use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def, duka_user_data};
use duka_shared::value::DukaInt;

use crate::{
    builtin::{
        arg::ok,
        regex::{Compiled, Runner, compile},
    },
    errors::DukaRuntimeError,
    value::{RuntimeDukaArray, RuntimeValue},
};

duka_builtin_def! {
    mod regex
    doc "Regex for duka"
    fn {
        meta: impl_search, impl_find_all, impl_compile
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
    #[duka_builtin(name = "search", params(self: userdata, text: string, from: int = 0), returns(vararg))]
    fn impl_search(&self, heap: &mut Heap, text: String, from: DukaInt) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
        let Some(m) = Runner::new(&self.inner).search(&text, from as usize) else {
            return Ok(vec![RuntimeValue::Bool(false)]);
        };
        let items = m
            .captures
            .into_iter()
            .map(|(from, to)| RuntimeValue::from_string(heap, text[from..to].to_string()))
            .collect();
        Ok(ok(RuntimeValue::Array(
            heap.alloc(GcCell::new(RuntimeDukaArray { items })),
        )))
    },
    #[duka_builtin(name = "find_all", params(self: userdata, text: string), returns(array))]
    fn impl_find_all(&self, heap: &mut Heap, text: String) -> Result<RuntimeValue, DukaRuntimeError> {
        let mut arrays = vec![];
        for m in Runner::new(&self.inner).find_all(&text) {
            let items = m
                .captures
                .into_iter()
                .map(|(from, to)| RuntimeValue::from_string(heap, text[from..to].to_string()))
                .collect();
            arrays.push(RuntimeValue::Array(heap.alloc(GcCell::new(RuntimeDukaArray { items }))));
        }
        Ok(RuntimeValue::Array(
            heap.alloc(GcCell::new(RuntimeDukaArray { items: arrays })),
        ))
    }
}

#[duka_builtin(doc = "Compile a pattern into CompiledRegex", params(pattern: string), returns(any))]
fn impl_compile(heap: &mut Heap, pattern: String) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(CompiledRegex::new(&pattern)?.into_value(heap))
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
    let reg = compile(&pattern).map_err(|e| DukaRuntimeError::Custom(e.to_string()))?;
    let Some(m) = Runner::new(&reg).search(&text, from as usize) else {
        return Ok(vec![RuntimeValue::Bool(false)]);
    };
    let items = m
        .captures
        .into_iter()
        .map(|(from, to)| RuntimeValue::from_string(heap, text[from..to].to_string()))
        .collect();
    // let nameds = m
    // .named_captures.into_iter()
    //     .map(|(name, (from, to))| RuntimeValue::from_string(heap, (&text[from..to]).to_string()))
    //     .collect();
    Ok(ok(RuntimeValue::Array(
        heap.alloc(GcCell::new(RuntimeDukaArray { items })),
    )))
}
#[duka_builtin(
    name = "find_all", 
    doc = "Find all strings by given pattern (global mode)",
    params(pattern: string, text: string),
    returns(array),
    return_doc = "Nested array, `[[captures1...], [captures2...]]`"
)]
fn impl_find_all(
    heap: &mut Heap,
    pattern: String,
    text: String,
) -> Result<RuntimeValue, DukaRuntimeError> {
    let reg = compile(&pattern).map_err(|e| DukaRuntimeError::Custom(e.to_string()))?;
    let mut arrays = vec![];
    for m in Runner::new(&reg).find_all(&text) {
        let items = m
            .captures
            .into_iter()
            .map(|(from, to)| RuntimeValue::from_string(heap, (&text[from..to]).to_string()))
            .collect();
        arrays.push(RuntimeValue::Array(
            heap.alloc(GcCell::new(RuntimeDukaArray { items })),
        ));
    }
    Ok(RuntimeValue::Array(
        heap.alloc(GcCell::new(RuntimeDukaArray { items: arrays })),
    ))
}
