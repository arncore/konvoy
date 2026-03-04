import * as vscode from 'vscode';
import { getKonvoyPath } from './konvoyBinary';
import { getOutputChannel, disposeOutputChannel } from './outputChannel';
import { watchManifests, getBestWorkspaceFolder } from './workspaceDetector';

const COMMAND_IDS = [
    'konvoy.build',
    'konvoy.buildRelease',
    'konvoy.run',
    'konvoy.runRelease',
    'konvoy.test',
    'konvoy.lint',
    'konvoy.update',
    'konvoy.clean',
    'konvoy.doctor',
    'konvoy.toolchainInstall',
    'konvoy.toolchainList',
] as const;

export function activate(context: vscode.ExtensionContext): void {
    const output = getOutputChannel();
    output.appendLine(`Konvoy extension activated (binary: ${getKonvoyPath()})`);

    // Watch for konvoy.toml creation/deletion
    const watcher = watchManifests(
        (uri) => output.appendLine(`Detected new manifest: ${uri.fsPath}`),
        (uri) => output.appendLine(`Manifest removed: ${uri.fsPath}`),
    );
    context.subscriptions.push(watcher);

    // Register stub commands -- will be replaced by the commands module
    for (const id of COMMAND_IDS) {
        const disposable = vscode.commands.registerCommand(id, () => {
            // TODO: replace with real command implementation (commands module)
            vscode.window.showInformationMessage(`${id}: not yet implemented`);
        });
        context.subscriptions.push(disposable);
    }

    // TODO: register task provider (taskProvider module)
    // TODO: register TOML support (tomlSupport module)
}

export function deactivate(): void {
    disposeOutputChannel();
}
