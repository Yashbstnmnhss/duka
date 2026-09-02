export class Bridge {
    constructor(wasmInstance) {
        this.instance = wasmInstance
        this.enc = new TextEncoder()
        this.dec = new TextDecoder()
        this.allocs = []
    }

    alloc(len) {
        const ptr = this.instance.exports.duka_alloc(len)
        this.allocs.push(ptr)
        return ptr
    }

    write(ptr, data) {
        new Uint8Array(this.instance.exports.memory.buffer, ptr, data.length).set(data)
    }

    freeAll() {
        this.instance.exports.duka_free()
        this.allocs.length = 0
    }

    readResult() {
        const ptr = this.instance.exports.duka_result_ptr()
        const len = this.instance.exports.duka_result_len()
        if (len === 0) return { ok: false, error: "empty result from WASM" }
        const bytes = new Uint8Array(this.instance.exports.memory.buffer, ptr, len)
        const text = this.dec.decode(bytes)
        try {
            return JSON.parse(text)
        } catch {
            return { ok: false, error: text || "failed to parse WASM result" }
        }
    }

    loadModules(modules) {
        for (const [name, data] of Object.entries(modules)) {
            const nameB = this.enc.encode(name)
            const dataB = data instanceof Uint8Array ? data : this.enc.encode(data)
            const ptr = this.alloc(nameB.length + dataB.length)
            this.write(ptr, nameB)
            this.write(ptr + nameB.length, dataB)
            this.instance.exports.duka_add_module(
                ptr,
                nameB.length,
                ptr + nameB.length,
                dataB.length,
            )
        }
    }

    async loadModulesFromUrl(modules) {
        const tasks = Object.entries(modules).map(async ([name, url]) => {
            const resp = await fetch(url)
            return [name, new Uint8Array(await resp.arrayBuffer())]
        })
        const entries = await Promise.all(tasks)
        const map = Object.fromEntries(entries)
        this.loadModules(map)
    }

    setEntry(entryName) {
        const bytes = this.enc.encode(entryName)
        const ptr = this.alloc(bytes.length)
        this.write(ptr, bytes)
        this.instance.exports.duka_set_entry(ptr, bytes.length)
    }

    setArgs(args) {
        const json = JSON.stringify(args)
        const bytes = this.enc.encode(json)
        const ptr = this.alloc(bytes.length)
        this.write(ptr, bytes)
        this.instance.exports.duka_set_args(ptr, bytes.length)
    }

    run(entryData) {
        const data = entryData instanceof Uint8Array ? entryData : this.enc.encode(entryData || '')
        const ptr = this.alloc(data.length)
        this.write(ptr, data)
        const code = this.instance.exports.duka_run(ptr, data.length)
        const result = this.readResult()
        this.freeAll()
        return { code, ...result }
    }

    resume() {
        const code = this.instance.exports.duka_resume()
        const result = this.readResult()
        this.freeAll()
        return { code, ...result }
    }

    handleEvent(id, data) {
        const idB = this.enc.encode(String(id))
        const dataB = this.enc.encode(typeof data === 'string' ? data : JSON.stringify(data))
        const idPtr = this.alloc(idB.length)
        const dataPtr = this.alloc(dataB.length)
        this.write(idPtr, idB)
        this.write(dataPtr, dataB)
        const code = this.instance.exports.duka_handle_event(
            idPtr,
            idB.length,
            dataPtr,
            dataB.length,
        )
        const result = this.readResult()
        this.freeAll()
        return { code, ...result }
    }
}

export async function loadWasm(wasmUrl) {
    const wasm = await fetch(wasmUrl)
    const { instance } = await WebAssembly.instantiateStreaming(wasm)
    return new Bridge(instance)
}
