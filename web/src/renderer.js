import { init, classModule, propsModule, styleModule, eventListenersModule } from "snabbdom"
import { jsonToVnode } from "./vdom.js"

const patchVnode = init([
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
            this.currentVnode = patchVnode(this.currentVnode, newVnode)
        } else {
            this.currentVnode = patchVnode(this.rootEl, newVnode)
        }
    }

    processPatches(patches) {
        if (!patches || patches.length === 0) return
        for (const p of patches) {
            if (p.op === "mount") {
                this.mount(p.selector)
            } else if (p.op === "render") {
                this.applyVnode(p.vnode)
            } else if (p.op === "patch") {
                this.applyPatch(p.selector, p.old_vnode, p.new_vnode)
            } else if (p.op === "unmount") {
                this.unmount()
            } else if (p.op === "inject_css") {
                this.injectCss(p.content)
            } else if (p.op === "inject_html") {
                this.injectHtml(p.selector, p.content)
            } else if (p.op === "log") {
                console.log("[duka]", p.message)
            }
        }
    }

    injectCss(content) {
        const style = document.createElement("style")
        style.textContent = content
        document.head.appendChild(style)
    }

    injectHtml(selector, content) {
        const el = document.querySelector(selector)
        if (el) el.innerHTML = content
    }

    applyPatch(selector, oldVnodeJson, newVnodeJson) {
        if (!this.rootEl) throw new Error("no mount target")
        const newVnode = jsonToVnode(newVnodeJson)
        if (this.currentVnode) {
            this.currentVnode = patchVnode(this.currentVnode, newVnode)
        } else {
            const oldVnode = jsonToVnode(oldVnodeJson)
            this.currentVnode = patchVnode(oldVnode, newVnode)
        }
    }

    unmount() {
        if (this.currentVnode) {
            patchVnode(this.currentVnode, { sel: "div", children: [] })
            this.currentVnode = null
        }
    }
}
