import * as vscode from 'vscode';
import { getKonvoyPath } from './konvoyBinary';
import { getOutputChannel, disposeOutputChannel } from './outputChannel';
import { watchManifests } from './workspaceDetector';
import { registerCommands } from './commands';
import { registerTaskProvider } from './taskProvider';
import { registerTomlSupport } from './tomlSupport';
import { disposeDiagnosticCollection } from './diagnostics';
import { initVariant } from './variantManager';

export function activate(context: vscode.ExtensionContext): void {
    const output = getOutputChannel();
    output.appendLine(`Konvoy extension activated (binary: ${getKonvoyPath()})`);

    // Watch for konvoy.toml creation/deletion
    const watcher = watchManifests(
        (uri) => output.appendLine(`Detected new manifest: ${uri.fsPath}`),
        (uri) => output.appendLine(`Manifest removed: ${uri.fsPath}`),
    );
    context.subscriptions.push(watcher);

    // Register commands
    const commandDisposables = registerCommands();
    for (const d of commandDisposables) {
        context.subscriptions.push(d);
    }

    // Initialize run variant toggle (debug / release)
    const variantDisposables = initVariant(context);
    for (const d of variantDisposables) {
        context.subscriptions.push(d);
    }

    // Register task provider
    context.subscriptions.push(registerTaskProvider());

    // Register TOML language support
    const tomlDisposables = registerTomlSupport(context);
    for (const d of tomlDisposables) {
        context.subscriptions.push(d);
    }
}

export function deactivate(): void {
    disposeDiagnosticCollection();
    disposeOutputChannel();
}
