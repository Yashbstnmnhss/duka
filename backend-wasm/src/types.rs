use duka_shared::ir::Constants;
use duka_shared::types::DebugInfo;

#[derive(Debug, Default)]
pub struct DukaWasmModule {
    pub constants: Constants,
    pub debug_info: DebugInfo,
    pub wasm_bytes: Vec<u8>,
    pub used_reg_count: usize,
    pub param_count: usize,
    pub has_var_arg: bool,
    pub nested_modules: Vec<DukaWasmModule>,
    pub up_indexes: Vec<usize>,
}
