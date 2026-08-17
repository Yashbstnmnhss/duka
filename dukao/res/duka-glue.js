// RUNTIME, MODULES, ENTRY constatns will be injected by rust code

export async function run(args = []) {
    const runtime = Uint8Array.from(atob(RUNTIME), c => c.charCodeAt(0))
    const { instance } = await WebAssembly.instantiate(runtime, {})
    const { exports } = instance
    const view = () => new Uint8Array(exports.memory.buffer)
    const enc = new TextEncoder()
    for (const [name, b64] of Object.entries(MODULES)) {
        const nameB = enc.encode(name)
        const dataB = Uint8Array.from(atob(b64), c => c.charCodeAt(0))
        const total = nameB.length + dataB.length
        const ptr = exports.duka_alloc(total) // Allocate memory for module import
        const buf = view()
        buf.set(nameB, ptr)
        buf.set(dataB, ptr + nameB.length)
        exports.duka_add_module(ptr, nameB.length, ptr + nameB.length, dataB.length)
    }
    const entry = Uint8Array.from(atob(MODULES[ENTRY]), c => c.charCodeAt(0))
    const eptr = exports.duka_alloc(entry.length) // Allocate memory for entry code
    view().set(entry, eptr)
    const entryName = enc.encode(ENTRY)
    const nptr = exports.duka_alloc(entryName.length)
    view().set(entryName, nptr)
    exports.duka_set_entry(nptr, entryName.length) // Set entrypoint
    const argB = enc.encode(JSON.stringify(args))
    const aptr = exports.duka_alloc(argB.length) // Pass arguments by json string
    view().set(argB, aptr)
    exports.duka_set_args(aptr, argB.length)
    const code = exports.duka_run(eptr, entry.length)
    const out = new Uint8Array(
        exports.memory.buffer,
        exports.duka_result_ptr(),
        exports.duka_result_len(),
    )
    const text = new TextDecoder().decode(out) // Get result
    if (code !== 0) throw new Error(text)
    return JSON.parse(text) // See duka-backend-wasm crate
}
