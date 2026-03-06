import * as vscode from 'vscode';

export type RunVariant = 'debug' | 'release';

const STATE_KEY = 'konvoy.runVariant';

let statusBarItem: vscode.StatusBarItem | undefined;
let currentVariant: RunVariant = 'debug';

export function initVariant(context: vscode.ExtensionContext): vscode.Disposable[] {
    const stored = context.workspaceState.get<RunVariant>(STATE_KEY);
    currentVariant = stored === 'release' ? 'release' : 'debug';

    vscode.commands.executeCommand('setContext', 'konvoy.releaseMode', currentVariant === 'release');

    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
    statusBarItem.command = 'konvoy.toggleRunVariant';
    updateStatusBar();
    statusBarItem.show();

    const toggleCmd = vscode.commands.registerCommand('konvoy.toggleRunVariant', () => {
        currentVariant = currentVariant === 'debug' ? 'release' : 'debug';
        context.workspaceState.update(STATE_KEY, currentVariant);
        vscode.commands.executeCommand('setContext', 'konvoy.releaseMode', currentVariant === 'release');
        updateStatusBar();
    });

    return [statusBarItem, toggleCmd];
}

function updateStatusBar(): void {
    if (!statusBarItem) { return; }
    if (currentVariant === 'debug') {
        statusBarItem.text = '$(debug-alt) Debug';
        statusBarItem.tooltip = 'Konvoy: Run in Debug mode (click to toggle)';
    } else {
        statusBarItem.text = '$(play) Release';
        statusBarItem.tooltip = 'Konvoy: Run in Release mode (click to toggle)';
    }
}

/** @internal Exposed for testing only. */
export const _testing = {
    getRunVariant: (): RunVariant => currentVariant,
    getStatusBarItem: (): vscode.StatusBarItem | undefined => statusBarItem,
    resetState: (): void => {
        currentVariant = 'debug';
        statusBarItem = undefined;
    },
};
