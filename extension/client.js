'use strict'

const path = require('path')
const fs = require('fs')
const vscode = require('vscode')
const { workspace, window } = vscode
const { LanguageClient, State } = require('vscode-languageclient/node')

let client
let channel
let extensionContext
let deactivating = false
let restartTimer
let restartDelay = 1000

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
function declaredFiles() {
    return {
        provideTextDocumentContent(uri) {
            const current = client
            if (!current) {
                return Promise.resolve('')
            }
            return current.sendRequest('duka/declaredFile', { uri: uri.toString() })
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

function isFile(file) {
    try {
        return fs.statSync(file).isFile()
    } catch {
        return false
    }
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

function candidatesFromPath() {
    const found = []
    for (const dir of (process.env.PATH || '').split(path.delimiter)) {
        if (dir) {
            found.push(path.join(dir, EXE))
        }
    }
    return found
}

function findServer() {
    const configured = workspace.getConfiguration('duka').get('lsp.path')
    if (configured) {
        log(`duka.lsp.path = ${configured}`)
        if (isFile(configured)) {
            return configured
        }
        log('duka.lsp.path does not exist, falling back to discovery')
    }

    const roots = []
    for (const folder of workspace.workspaceFolders || []) {
        roots.push(folder.uri.fsPath)
    }
    if (extensionContext && extensionContext.extensionPath) {
        roots.push(extensionContext.extensionPath)
    }
    log(`search roots: ${roots.length ? roots.join(', ') : '<none: no folder open>'}`)
    for (const root of roots) {
        for (const candidate of candidatesFrom(root)) {
            if (isFile(candidate)) {
                log(`discovered ${candidate}`)
                return candidate
            }
        }
    }
    for (const candidate of candidatesFromPath()) {
        if (isFile(candidate)) {
            log(`discovered in PATH: ${candidate}`)
            return candidate
        }
    }
    return null
}

function stopClient() {
    if (restartTimer) {
        clearTimeout(restartTimer)
        restartTimer = undefined
    }
    const running = client
    client = undefined
    if (!running) {
        return Promise.resolve()
    }
    return running
        .stop()
        .catch((err) => log(`stop failed: ${err && err.message ? err.message : err}`))
}

function scheduleRestart(reason) {
    if (deactivating || restartTimer) {
        return
    }
    log(`restarting in ${restartDelay}ms (${reason})`)
    restartTimer = setTimeout(() => {
        restartTimer = undefined
        stopClient()
            .then(() => startClient())
            .then(() => {
                restartDelay = 1000
            })
            .catch((err) => {
                log(`restart failed: ${err && err.message ? err.message : err}`)
                restartDelay = Math.min(restartDelay * 2, 15000)
                scheduleRestart('restart failed')
            })
    }, restartDelay)
}

async function startClient() {
    const serverPath = findServer()
    if (!serverPath) {
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
        documentSelector: [
            { scheme: 'file', language: 'duka' },
            { scheme: SCHEME, language: 'duka' },
        ],
        outputChannel: channel,
        synchronize: {
            fileEvents: [
                workspace.createFileSystemWatcher('**/*.duka'),
                workspace.createFileSystemWatcher('**/kao.toml'),
            ],
        },
        initializationOptions: {
            modulePaths: workspace.getConfiguration('duka').get('modulePaths') || [],
        },
    }

    const startedClient = new LanguageClient(
        'dukaLanguage',
        'Duka Language',
        serverOptions,
        clientOptions,
    )
    client = startedClient
    startedClient.onDidChangeState((event) => {
        log(`state -> ${State[event.newState]}`)
        if (
            event.newState === State.Stopped &&
            !deactivating &&
            client === startedClient
        ) {
            scheduleRestart('server stopped')
        }
    })

    const started = Date.now()
    await startedClient.start()
    log(`client ready in ${Date.now() - started}ms`)
    window.setStatusBarMessage('Duka: language server connected', 5000)
}

function activate(context) {
    channel = window.createOutputChannel('Duka Language')
    context.subscriptions.push(channel)
    extensionContext = context
    deactivating = false
    log('activate() called')

    context.subscriptions.push(
        workspace.registerTextDocumentContentProvider(SCHEME, declaredFiles()),
    )
    context.subscriptions.push(
        workspace.onDidChangeConfiguration((event) => {
            if (
                event.affectsConfiguration('duka.lsp.path') ||
                event.affectsConfiguration('duka.modulePaths')
            ) {
                log('configuration changed, restarting')
                scheduleRestart('configuration changed')
            }
        }),
    )

    startClient().catch((err) => {
        const message = err && err.message ? err.message : String(err)
        log(`start failed: ${message}`)
        window.showErrorMessage(`Failed to start duka-lsp: ${message}`)
    })
}

function deactivate() {
    deactivating = true
    return stopClient()
}

module.exports = { activate, deactivate }
