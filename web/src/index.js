import { loadWasm } from "./bridge.js"
import { Renderer } from "./renderer.js"
import { setupEvents } from "./events.js"

export async function createApp(config) {
    const bridge = await loadWasm(config.wasm)
    await bridge.loadModulesFromUrl(config.modules)
    bridge.setEntry(config.entry)
    bridge.setArgs(config.args || [])

    const entryUrl = config.modules[config.entry]
    const entryResp = await fetch(entryUrl)
    const entryBytes = new Uint8Array(await entryResp.arrayBuffer())

    const renderer = new Renderer()
    let cleanupEvents = null

    function handleResponse(resp) {
        if (!resp.ok) throw new Error(resp.error)
        renderer.processPatches(resp.patches)
    }

    return {
        mount(selector) {
            renderer.mount(selector)
            cleanupEvents = setupEvents(renderer.rootEl, (id, data) => {
                const resp = bridge.handleEvent(id, data)
                handleResponse(resp)
            })
            return this
        },

        run() {
            const resp = bridge.run(entryBytes)
            handleResponse(resp)
            return this
        },

        unmount() {
            if (cleanupEvents) cleanupEvents()
            renderer.unmount()
        },

        get bridge() { return bridge },
        get renderer() { return renderer },
    }
}
