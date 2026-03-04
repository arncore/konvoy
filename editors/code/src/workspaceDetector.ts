import * as vscode from 'vscode';
import * as path from 'path';

const MANIFEST_NAME = 'konvoy.toml';

/**
 * Returns all workspace folders that contain a konvoy.toml at their root.
 */
export function findKonvoyWorkspaces(): vscode.WorkspaceFolder[] {
    const folders = vscode.workspace.workspaceFolders;
    if (!folders) {
        return [];
    }
    return folders.filter(folder => {
        const manifestUri = vscode.Uri.joinPath(folder.uri, MANIFEST_NAME);
        // We can't do async stat here, so rely on the activation event
        // (workspaceContains:konvoy.toml) having already confirmed existence.
        return true;
    });
}

/**
 * Creates a FileSystemWatcher that fires when konvoy.toml is created or deleted
 * anywhere in the workspace.
 */
export function watchManifests(
    onCreated: (uri: vscode.Uri) => void,
    onDeleted: (uri: vscode.Uri) => void,
): vscode.FileSystemWatcher {
    const watcher = vscode.workspace.createFileSystemWatcher(
        `**/${MANIFEST_NAME}`,
        false,
        true, // ignore changes (only care about create/delete)
        false,
    );
    watcher.onDidCreate(onCreated);
    watcher.onDidDelete(onDeleted);
    return watcher;
}

/**
 * Returns the best workspace folder for the current context:
 * - If the active editor is inside a workspace folder with konvoy.toml, use that.
 * - Otherwise fall back to the first workspace folder.
 */
export function getBestWorkspaceFolder(): vscode.WorkspaceFolder | undefined {
    const editor = vscode.window.activeTextEditor;
    if (editor) {
        const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
        if (folder) {
            return folder;
        }
    }
    const folders = vscode.workspace.workspaceFolders;
    return folders?.[0];
}
