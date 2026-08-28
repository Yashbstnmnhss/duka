import { h } from "snabbdom"

function parseClass(val) {
    if (!val) return {}
    if (typeof val === "string") {
        return Object.fromEntries(val.split(/\s+/).filter(Boolean).map(k => [k, true]))
    }
    return val
}

function parseStyle(val) {
    if (!val) return {}
    return val
}

function parseChildren(children) {
    if (!children) return []
    return children.map(vnode)
}

function vnode(json) {
    if (json === null || json === undefined) return ""
    if (typeof json === "string") return json
    if (typeof json === "number") return String(json)
    if (typeof json === "boolean") return ""

    const { tag, key, props, children } = json
    if (!tag) return parseChildren(children || [])

    const data = {}
    const listeners = {}

    if (props) {
        for (const [k, v] of Object.entries(props)) {
            if (k === "class") {
                data.class = parseClass(v)
            } else if (k === "style") {
                data.style = parseStyle(v)
            } else if (k.startsWith("on")) {
                const evtName = k.slice(2).toLowerCase()
                if (typeof v === "string" && v.startsWith("$evt:")) {
                    data["data-duka-evt"] = v.slice(5)
                } else if (typeof v === "function") {
                    listeners[evtName] = v
                }
            } else {
                if (!data.attrs) data.attrs = {}
                data.attrs[k] = v
            }
        }
    }

    if (key) data.key = key

    if (Object.keys(listeners).length > 0) {
        data.on = listeners
    }

    return h(tag, data, parseChildren(children))
}

export function jsonToVnode(json) {
    return vnode(json)
}
