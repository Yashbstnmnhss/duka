use crate::{DukaWasmError, DukaWasmModule};
use duka_shared::{ir::DukaIR, types::DukaGenerator};
use walrus;
use walrus::{FunctionBuilder, Module};

pub struct WasmGenerator {
    module: Module,
}
impl Default for WasmGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmGenerator {
    pub fn new() -> Self {
        WasmGenerator {
            module: Default::default(),
        }
    }

    pub fn gen_mod(&mut self, _ir: DukaIR) -> Result<DukaWasmModule, DukaWasmError> {
        let mut builder = FunctionBuilder::new(&mut self.module.types, &[], &[]);

        builder.func_body();

        let built = builder.finish(vec![], &mut self.module.funcs);
        self.module.exports.add("main", built);
        let bytes = self.module.emit_wasm();
        Ok(DukaWasmModule {
            wasm_bytes: bytes,
            ..Default::default()
        })
    }
}

impl DukaGenerator<DukaWasmModule, DukaWasmError> for WasmGenerator {
    type InputType = DukaIR;
    type ConfigType = ();

    fn generate(
        input: Self::InputType,
        _: Self::ConfigType,
    ) -> Result<DukaWasmModule, DukaWasmError> {
        Self::new().gen_mod(input)
    }
}
