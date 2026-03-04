import * as vscode from 'vscode';

export function getKonvoyPath(): string {
    const config = vscode.workspace.getConfiguration('konvoy');
    const configPath = config.get<string>('path', '');
    return configPath || 'konvoy';
}
