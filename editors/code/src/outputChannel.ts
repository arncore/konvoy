import * as vscode from 'vscode';

let channel: vscode.OutputChannel | undefined;

export function getOutputChannel(): vscode.OutputChannel {
    if (!channel) {
        channel = vscode.window.createOutputChannel('Konvoy');
    }
    return channel;
}

export function disposeOutputChannel(): void {
    channel?.dispose();
    channel = undefined;
}
