// RUNTIME, MODULES, ENTRY constants will be injected by rust code

export async function run(args = []) {
    const wasm = await fetch(__RUNTIME)
    const { instance } = await WebAssembly.instantiateStreaming(wasm)
    const { exports } = instance
    const view = () => new Uint8Array(exports.memory.buffer)
    const enc = new TextEncoder()

    const allocated = []

    function alloc(len) {
        const ptr = exports.duka_alloc(len)
        allocated.push(ptr)
        return ptr
    }
    function freeAll() {
        exports.duka_free()
        allocated.length = 0
    }
    try {
        for (const [name, url] of Object.entries(__MODULES)) {
            const nameB = enc.encode(name)
            const resp = await fetch(url)
            const dataB = new Uint8Array(await resp.arrayBuffer())

            const total = nameB.length + dataB.length
            const ptr = alloc(total) // Allocate memory for module import
            const buf = view()
            buf.set(nameB, ptr)
            buf.set(dataB, ptr + nameB.length)
            exports.duka_add_module(ptr, nameB.length, ptr + nameB.length, dataB.length)
        }
        const resp = await fetch(__MODULES[__ENTRY])
        const entry = new Uint8Array(await resp.arrayBuffer())
        const eptr = alloc(entry.length) // Allocate memory for entry code
        view().set(entry, eptr)
        const entryName = enc.encode(__ENTRY)
        const nptr = alloc(entryName.length)
        view().set(entryName, nptr)
        exports.duka_set_entry(nptr, entryName.length) // Set entrypoint
        const argB = enc.encode(JSON.stringify(args))
        const aptr = alloc(argB.length) // Pass arguments by json string
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
    } finally {
        freeAll()
    }
}
