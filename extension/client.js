'use strict';

const path = require('path');
const fs = require('fs');
const { workspace, window } = require('vscode');
const { LanguageClient } = require('vscode-languageclient/node');

let client;

function findServer() {
  const configured = workspace.getConfiguration('duka').get('lsp.path');
  if (configured) {
    return configured;
  }
  const exe = process.platform === 'win32' ? 'duka-lsp.exe' : 'duka-lsp';
  const folders = workspace.workspaceFolders || [];
  for (const folder of folders) {
    for (const dir of ['target/debug', 'target/release']) {
      const candidate = path.join(folder.uri.fsPath, dir, exe);
      if (fs.existsSync(candidate)) {
        return candidate;
      }
    }
  }
  return null;
}

async function activate() {
  const serverPath = findServer();
  if (!serverPath) {
    window.showErrorMessage(
      'duka-lsp executable not found. Build it with "cargo build -p duka-lsp" or set "duka.lsp.path".'
    );
    return;
  }

  const serverOptions = {
    run: { command: serverPath },
    debug: { command: serverPath },
  };
  const clientOptions = {
    documentSelector: [{ scheme: 'file', language: 'duka' }],
  };

  client = new LanguageClient('dukaLanguage', 'Duka Language', serverOptions, clientOptions);
  client.start();
}

function deactivate() {
  if (!client) {
    return undefined;
  }
  return client.stop();
}

module.exports = { activate, deactivate };
