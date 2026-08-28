const DELEGATE_EVENTS = [
    "click", "dblclick", "mousedown", "mouseup", "mousemove",
    "keydown", "keyup", "keypress",
    "input", "change", "submit",
    "focus", "blur",
    "touchstart", "touchend", "touchmove",
]

export function setupEvents(rootEl, onEvent) {
    const handler = (e) => {
        let target = e.target
        while (target && target !== rootEl) {
            const id = target.dataset?.dukaEvt
            if (id) {
                const value = extractValue(e, target)
                onEvent(id, { type: e.type, value })
                return
            }
            target = target.parentElement
        }
    }

    for (const evt of DELEGATE_EVENTS) {
        rootEl.addEventListener(evt, handler, evt === "focus" || evt === "blur" ? undefined : { passive: true })
    }

    return () => {
        for (const evt of DELEGATE_EVENTS) {
            rootEl.removeEventListener(evt, handler)
        }
    }
}

function extractValue(e, target) {
    const tag = target.tagName?.toLowerCase()
    if (tag === "input") {
        if (target.type === "checkbox" || target.type === "radio") {
            return target.checked
        }
        return target.value
    }
    if (tag === "select") {
        return target.value
    }
    if (tag === "textarea") {
        return target.value
    }
    if (e.type === "keydown" || e.type === "keyup" || e.type === "keypress") {
        return e.key
    }
    return ""
}
