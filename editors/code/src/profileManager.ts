import * as vscode from 'vscode';

export type RunProfile = 'debug' | 'release';

const STATE_KEY = 'konvoy.runProfile';

let statusBarItem: vscode.StatusBarItem | undefined;
let currentProfile: RunProfile = 'debug';

export function initProfile(context: vscode.ExtensionContext): vscode.Disposable[] {
    const stored = context.workspaceState.get<RunProfile>(STATE_KEY);
    currentProfile = stored === 'release' ? 'release' : 'debug';

    vscode.commands.executeCommand('setContext', 'konvoy.releaseMode', currentProfile === 'release');

    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
    statusBarItem.command = 'konvoy.toggleRunProfile';
    updateStatusBar();
    statusBarItem.show();

    const toggleCmd = vscode.commands.registerCommand('konvoy.toggleRunProfile', () => {
        currentProfile = currentProfile === 'debug' ? 'release' : 'debug';
        context.workspaceState.update(STATE_KEY, currentProfile);
        vscode.commands.executeCommand('setContext', 'konvoy.releaseMode', currentProfile === 'release');
        updateStatusBar();
    });

    return [statusBarItem, toggleCmd];
}

function updateStatusBar(): void {
    if (!statusBarItem) { return; }
    if (currentProfile === 'debug') {
        statusBarItem.text = '$(debug-alt) Debug';
        statusBarItem.tooltip = 'Konvoy: Run in Debug mode (click to toggle)';
    } else {
        statusBarItem.text = '$(play) Release';
        statusBarItem.tooltip = 'Konvoy: Run in Release mode (click to toggle)';
    }
}

/** @internal Exposed for testing only. */
export const _testing = {
    getRunProfile: (): RunProfile => currentProfile,
    getStatusBarItem: (): vscode.StatusBarItem | undefined => statusBarItem,
    resetState: (): void => {
        currentProfile = 'debug';
        statusBarItem = undefined;
    },
};
