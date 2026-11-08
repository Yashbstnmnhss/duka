//! RUNTIME, MODULES, ENTRY, __wasmModule constants will be injected by rust code

const { DukaRuntime } = __wasmModule

export function createRuntime({ args = [], input = null } = {}) {
    const rt = new DukaRuntime()
    if (__ENTRY) rt.setEntry(__ENTRY)
    if (args.length) rt.setArgs(args)
    if (input != null) {
        rt.setInput(input instanceof Uint8Array ? input : new TextEncoder().encode(input))
    }
    return rt
}

export async function loadModules(rt, baseUrl = '') {
    await Promise.all(
        Object.entries(__MODULES).map(async ([name, url]) => {
            const resp = await fetch(baseUrl + url)
            if (!resp.ok) throw new Error(`module "${name}" fetch failed: ${resp.status}`)
            const bytes = new Uint8Array(await resp.arrayBuffer())
            rt.addModule(name, bytes)
        }),
    )
}

export { __RUNTIME as wasm, __MODULES as modules, __ENTRY as entry, DukaRuntime }
