import { init, classModule, propsModule, styleModule, eventListenersModule } from "snabbdom"
import { jsonToVnode } from "./vdom.js"

const patch = init([
    classModule,
    propsModule,
    styleModule,
    eventListenersModule,
])

export class Renderer {
    constructor() {
        this.rootEl = null
        this.currentVnode = null
    }

    mount(selector) {
        this.rootEl = document.querySelector(selector)
        if (!this.rootEl) throw new Error(`mount target not found: ${selector}`)
    }

    applyVnode(vnodeJson) {
        const newVnode = jsonToVnode(vnodeJson)
        if (!this.rootEl) throw new Error("no mount target")
        if (this.currentVnode) {
            this.currentVnode = patch(this.currentVnode, newVnode)
        } else {
            this.currentVnode = patch(this.rootEl, newVnode)
        }
    }

    processPatches(patches) {
        if (!patches || patches.length === 0) return
        for (const p of patches) {
            if (p.op === "mount") {
                this.mount(p.selector)
            } else if (p.op === "render") {
                this.applyVnode(p.vnode)
            } else if (p.op === "unmount") {
                this.unmount()
            } else if (p.op === "log") {
                console.log("[duka]", p.message)
            }
        }
    }

    unmount() {
        if (this.currentVnode) {
            patch(this.currentVnode, { sel: "div", children: [] })
            this.currentVnode = null
        }
    }
}
