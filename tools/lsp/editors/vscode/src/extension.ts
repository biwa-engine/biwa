import * as path from 'path';
import * as fs from 'fs';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
    const serverBin = serverBinaryPath(context);

    if (!fs.existsSync(serverBin)) {
        vscode.window.showErrorMessage(
            `biwa-lsp binary not found at: ${serverBin}\n` +
            `Run: cargo build --release -p biwa-lsp-server && ` +
            `cp target/release/biwa-lsp editors/vscode/bin/biwa-lsp`
        );
        return;
    }

    const serverOptions: ServerOptions = {
        run: {
            command: serverBin,
            transport: TransportKind.stdio,
        },
        debug: {
            command: serverBin,
            transport: TransportKind.stdio,
            options: {
                env: { ...process.env, RUST_LOG: 'debug' },
            },
        },
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'biwa' }],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/*.biwa'),
        },
        outputChannelName: 'Biwa Language Server',
    };

    client = new LanguageClient(
        'biwa-lsp',
        'Biwa Language Server',
        serverOptions,
        clientOptions,
    );

    client.start();
    context.subscriptions.push(client);
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}

function serverBinaryPath(context: vscode.ExtensionContext): string {
    const binaryName = process.platform === 'win32' ? 'biwa-lsp.exe' : 'biwa-lsp';
    return context.asAbsolutePath(path.join('bin', binaryName));
}
