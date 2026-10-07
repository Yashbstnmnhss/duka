'use strict'

const path = require('path')
const fs = require('fs')
const vscode = require('vscode')
const { workspace, window } = vscode
const { LanguageClient, State } = require('vscode-languageclient/node')

let client
let channel

const EXE = process.platform === 'win32' ? 'duka-lsp.exe' : 'duka-lsp'

/**
 * The scheme the server generates declaration files under. A builtin is written
 * in Rust, so there is no file for the editor to open; the server answers
 * `duka/declaredFile` with the text and nothing is written to disk.
 */
const SCHEME = 'duka-virtual'

/**
 * Serves those generated files.
 *
 * VSCode asks its provider for the text before it opens the document, so the
 * document the reader ends up looking at is a real one: the server analyses it
 * like any other, and hover, completion and goto work inside it.
 */
function declaredFiles(client) {
    return {
        provideTextDocumentContent(uri) {
            // An object, because a bare string is not a named parameter: the JSON-RPC
            // layer wraps anything that is not an object literal in an array, and the
            // server would be asked to read a list where a uri is.
            return client.sendRequest('duka/declaredFile', { uri: uri.toString() })
        },
    }
}

function log(line) {
    const stamp = new Date().toISOString().slice(11, 23)
    if (channel) {
        channel.appendLine(`[${stamp}] ${line}`)
    }
    console.log(`[duka] ${line}`)
}

function candidatesFrom(root) {
    const found = []
    let dir = root
    for (let depth = 0; depth < 8; depth++) {
        found.push(path.join(dir, 'target', 'debug', EXE))
        found.push(path.join(dir, 'target', 'release', EXE))
        const parent = path.dirname(dir)
        if (parent === dir) {
            break
        }
        dir = parent
    }
    return found
}

function findServer() {
    const configured = workspace.getConfiguration('duka').get('lsp.path')
    if (configured) {
        log(`duka.lsp.path = ${configured}`)
        if (fs.existsSync(configured)) {
            return configured
        }
        log('duka.lsp.path does not exist, falling back to discovery')
    }

    const roots = []
    for (const folder of workspace.workspaceFolders || []) {
        roots.push(folder.uri.fsPath)
    }
    if (vscode.context && vscode.context.extensionPath) {
        roots.push(vscode.context.extensionPath)
    }
    log(`search roots: ${roots.length ? roots.join(', ') : '<none: no folder open>'}`)
    for (const root of roots) {
        for (const candidate of candidatesFrom(root)) {
            if (fs.existsSync(candidate)) {
                log(`discovered ${candidate}`)
                return candidate
            }
        }
    }
    return null
}

function activate(context) {
    channel = window.createOutputChannel('Duka Language')
    context.subscriptions.push(channel)
    log('activate() called')

    const serverPath = findServer()
    if (!serverPath) {
        log('duka-lsp executable not found in any target/debug or target/release above the roots')
        window.showErrorMessage(
            'duka-lsp executable not found. Build it with "cargo build -p duka-lsp" or set "duka.lsp.path".',
        )
        return
    }

    const serverOptions = {
        run: { command: serverPath },
        debug: { command: serverPath },
    }
    const clientOptions = {
        // The generated declaration files are documents of this language too, so
        // the server has to be answering for them and not only for files on disk.
        documentSelector: [
            { scheme: 'file', language: 'duka' },
            { scheme: SCHEME, language: 'duka' },
        ],
        outputChannel: channel,
    }

    client = new LanguageClient('dukaLanguage', 'Duka Language', serverOptions, clientOptions)
    context.subscriptions.push(
        vscode.workspace.registerTextDocumentContentProvider(SCHEME, declaredFiles(client)),
    )
    client.onDidChangeState((event) => {
        log(`state -> ${State[event.newState]}`)
    })

    const started = Date.now()
    client
        .start()
        .then(() => {
            log(`client ready in ${Date.now() - started}ms`)
            window.setStatusBarMessage('Duka: language server connected', 5000)
        })
        .catch((err) => {
            const message = err && err.message ? err.message : String(err)
            log(`start failed after ${Date.now() - started}ms: ${message}`)
            window.showErrorMessage(`Failed to start duka-lsp (${serverPath}): ${message}`)
            client = undefined
        })
}

function deactivate() {
    if (!client) {
        return undefined
    }
    const running = client
    client = undefined
    return running.stop()
}

module.exports = { activate, deactivate }
