/* tslint:disable */
/* eslint-disable */

export class DukaRuntime {
    free(): void;
    [Symbol.dispose](): void;
    add_module(name: string, data: Uint8Array | number[]): void;
    clear_modules(): void;
    compile(source: string): any;
    format_version(): number;
    constructor();
    reset(): void;
    resume(): any;
    run(binary: Uint8Array | number[]): any;
    run_source(source: string): any;
    set_args(args: any): void;
    set_entry(name: string): void;
    set_input(data: Uint8Array | number[]): void;
    take_stderr(): string;
    /**
     * Everything the program wrote to standard output, drained.
     *
     * `run`/`run_source` hand back returned values only; a script whose whole
     * point is `print` would look empty without this.
     */
    take_stdout(): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_dukaruntime_free: (a: number, b: number) => void;
    readonly dukaruntime_add_module: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly dukaruntime_clear_modules: (a: number) => void;
    readonly dukaruntime_compile: (a: number, b: number, c: number) => [number, number, number];
    readonly dukaruntime_format_version: (a: number) => number;
    readonly dukaruntime_new: () => number;
    readonly dukaruntime_reset: (a: number) => void;
    readonly dukaruntime_resume: (a: number) => [number, number, number];
    readonly dukaruntime_run: (a: number, b: number, c: number) => [number, number, number];
    readonly dukaruntime_run_source: (a: number, b: number, c: number) => [number, number, number];
    readonly dukaruntime_set_args: (a: number, b: any) => [number, number];
    readonly dukaruntime_set_entry: (a: number, b: number, c: number) => void;
    readonly dukaruntime_set_input: (a: number, b: number, c: number) => void;
    readonly dukaruntime_take_stderr: (a: number) => [number, number];
    readonly dukaruntime_take_stdout: (a: number) => [number, number];
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
