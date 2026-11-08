#[cfg(feature = "frontend")]
use duka_lib::codegen::binary::Dump;
use duka_lib::{
    DukaVM,
    codegen::binary::{DukaBinary, FORMAT_VERSION, Load},
    duka_gc::Heap,
    duka_shared::types::ValueCount,
    value::RuntimeValue,
    vm::{VM, coroutine::OutputCell},
};
use js_sys::{Array, Object, Reflect};
use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, Mutex},
};
use wasm_bindgen::{JsError, JsValue, prelude::wasm_bindgen};

pub fn duka_to_js(val: &RuntimeValue) -> Result<JsValue, JsError> {
    Ok(match val {
        RuntimeValue::Nil => JsValue::null(),
        RuntimeValue::Int(i) => (*i).into(),
        RuntimeValue::Float(f) => (*f).into(),
        RuntimeValue::Bool(b) => (*b).into(),
        rv if rv.is_string() => rv.eval_to_string().to_string().into(),
        RuntimeValue::Table(gc) => {
            let b = gc.borrow();
            let obj = Object::new();
            for (k, v) in &b.inner {
                let _ = Reflect::set(&obj, &duka_to_js(k)?, &duka_to_js(v)?);
            }
            obj.into()
        }
        RuntimeValue::Array(gc) => {
            let b = gc.borrow();
            let arr = Array::new_with_length(b.len() as u32);
            for (i, v) in b.items.iter().enumerate() {
                arr.set(i as u32, duka_to_js(v)?);
            }
            arr.into()
        }
        v => v.name().into(),
    })
}

fn take_result(vm: &mut VM) -> Result<JsValue, JsError> {
    match vm
        .scheduler
        .main_mut()
        .inner
        .take_stack_many(0, ValueCount::VarArg)
    {
        Ok(vals) => {
            let arr = Array::new_with_length(vals.len() as u32);
            for (i, v) in vals.iter().enumerate() {
                arr.set(i as u32, duka_to_js(v)?);
            }
            Ok(arr.into())
        }
        Err(e) => Err(JsError::new(&format!("error: {e}"))),
    }
}

#[wasm_bindgen]
pub struct DukaRuntime {
    vm: Option<VM>,
    modules: HashMap<String, Vec<u8>>,
    entry: Option<String>,
    args: Vec<String>,
    input: Vec<u8>,
    /// What the program wrote to the standard streams. Kept here rather than
    /// inside the VM so a caller can read it after `run` has handed the value
    /// back -- a playground showing only returned values would miss `print`.
    stdout: Option<OutputCell>,
    stderr: Option<OutputCell>,
}

#[wasm_bindgen]
impl DukaRuntime {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        console_error_panic_hook::set_once();
        Self {
            vm: None,
            modules: HashMap::new(),
            entry: None,
            args: Vec::new(),
            input: Vec::new(),
            stdout: None,
            stderr: None,
        }
    }

    #[inline]
    pub fn format_version(&self) -> u32 {
        FORMAT_VERSION as u32
    }

    pub fn add_module(&mut self, name: &str, data: &[u8]) {
        self.modules.insert(name.to_string(), data.to_vec());
    }

    pub fn clear_modules(&mut self) {
        self.modules.clear();
    }

    pub fn set_entry(&mut self, name: &str) {
        self.entry = Some(name.to_string());
    }

    pub fn set_args(&mut self, args: JsValue) -> Result<(), JsError> {
        let v: Vec<String> = serde_wasm_bindgen::from_value(args)
            .map_err(|e| JsError::new(&format!("invalid args: {e}")))?;
        self.args = v;
        Ok(())
    }

    pub fn set_input(&mut self, data: &[u8]) {
        self.input = data.to_vec();
    }

    pub fn run(&mut self, binary: &[u8]) -> Result<JsValue, JsError> {
        self.vm = None;

        let proto = DukaBinary::load(&mut Cursor::new(binary))
            .map_err(|e| JsError::new(&format!("binary load error: {e}")))?
            .into_proto();

        let mut vm = self.init_vm()?;
        vm.execute(&proto)
            .map_err(|e| JsError::new(&format!("runtime error: {e}")))?;

        let result = take_result(&mut vm);
        self.vm = Some(vm);

        result
    }

    pub fn resume(&mut self) -> Result<JsValue, JsError> {
        let vm = self
            .vm
            .as_mut()
            .ok_or_else(|| JsError::new("VM not initialized: call run() first"))?;

        vm.scheduler
            .go(&mut vm.heap)
            .map_err(|e| JsError::new(&format!("runtime error: {e}")))?;

        take_result(vm)
    }

    pub fn reset(&mut self) {
        self.vm = None;
        self.entry = None;
        self.args.clear();
        self.input.clear();
        self.stdout = None;
        self.stderr = None;
    }

    /// Everything the program wrote to standard output, drained.
    ///
    /// `run`/`run_source` hand back returned values only; a script whose whole
    /// point is `print` would look empty without this.
    pub fn take_stdout(&self) -> String {
        drain_cell(&self.stdout)
    }

    pub fn take_stderr(&self) -> String {
        drain_cell(&self.stderr)
    }

    #[cfg(feature = "frontend")]
    pub fn compile(&self, source: &str) -> Result<JsValue, JsError> {
        use duka_lib::duka_shared::types::SourceName;

        let proto = duka_lib::module::from_source(source, SourceName::Unnamed, Default::default())
            .map_err(|e| JsError::new(&e.to_string()))?;
        let mut buffer = vec![];
        proto
            .dump(&mut buffer)
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(js_sys::Uint8Array::from(&buffer[..]).into())
    }

    #[cfg(feature = "frontend")]
    pub fn run_source(&mut self, source: &str) -> Result<JsValue, JsError> {
        use duka_lib::duka_shared::types::SourceName;
        self.vm = None;

        let proto = duka_lib::module::from_source(source, SourceName::Unnamed, Default::default())
            .map_err(|e| JsError::new(&e.to_string()))?;

        let mut vm = self.init_vm()?;
        vm.execute(&proto)
            .map_err(|e| JsError::new(&format!("runtime error: {e}")))?;

        let result = take_result(&mut vm);
        self.vm = Some(vm);

        result
    }
}

fn drain_cell(cell: &Option<OutputCell>) -> String {
    let Some(cell) = cell else {
        return String::new();
    };
    let bytes = cell
        .lock()
        .map(|mut guard| std::mem::take(&mut *guard))
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

impl DukaRuntime {
    fn init_vm(&mut self) -> Result<VM, JsError> {
        let globals = duka_lib::builtin::require::default_vm_globals();
        duka_lib::builtin::require::set_loader(
            &globals,
            duka_lib::module::memory_loader(Arc::new(self.modules.clone()), "modules"),
        );

        let mut vm = VM::with_vm_globals(Heap::new(), globals);

        if let Some(entry) = &self.entry {
            vm.set_entry_path(std::path::PathBuf::from(entry));
        }

        let args: Vec<_> = self
            .args
            .iter()
            .map(|a| RuntimeValue::from_string(&mut vm.heap, a.clone()))
            .collect();
        vm.set_main_args(&args);

        let stdout: OutputCell = Arc::new(Mutex::new(vec![]));
        let stderr: OutputCell = Arc::new(Mutex::new(vec![]));
        self.stdout = Some(stdout.clone());
        self.stderr = Some(stderr.clone());
        vm.set_stdout(Some(stdout));
        vm.set_stderr(Some(stderr));

        let stdin: OutputCell = Arc::new(Mutex::new(self.input.clone()));
        vm.set_input(Some(stdin));

        Ok(vm)
    }
}
