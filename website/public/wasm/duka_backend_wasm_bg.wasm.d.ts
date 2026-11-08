/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export const __wbg_dukaruntime_free: (a: number, b: number) => void;
export const dukaruntime_add_module: (a: number, b: number, c: number, d: number, e: number) => void;
export const dukaruntime_clear_modules: (a: number) => void;
export const dukaruntime_compile: (a: number, b: number, c: number) => [number, number, number];
export const dukaruntime_format_version: (a: number) => number;
export const dukaruntime_new: () => number;
export const dukaruntime_reset: (a: number) => void;
export const dukaruntime_resume: (a: number) => [number, number, number];
export const dukaruntime_run: (a: number, b: number, c: number) => [number, number, number];
export const dukaruntime_run_source: (a: number, b: number, c: number) => [number, number, number];
export const dukaruntime_set_args: (a: number, b: any) => [number, number];
export const dukaruntime_set_entry: (a: number, b: number, c: number) => void;
export const dukaruntime_set_input: (a: number, b: number, c: number) => void;
export const dukaruntime_set_seed: (a: number, b: number) => void;
export const dukaruntime_take_stderr: (a: number) => [number, number];
export const dukaruntime_take_stdout: (a: number) => [number, number];
export const __wbindgen_malloc: (a: number, b: number) => number;
export const __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
export const __wbindgen_free: (a: number, b: number, c: number) => void;
export const __wbindgen_exn_store: (a: number) => void;
export const __externref_table_alloc: () => number;
export const __wbindgen_externrefs: WebAssembly.Table;
export const __externref_table_dealloc: (a: number) => void;
export const __wbindgen_start: () => void;
