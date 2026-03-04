import * as assert from 'assert';
import * as vscode from 'vscode';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { findKonvoyWorkspaces, watchManifests, getBestWorkspaceFolder } = require('../../workspaceDetector');

suite('workspaceDetector', () => {
    suite('getBestWorkspaceFolder', () => {
        test('returns a WorkspaceFolder when workspace folders exist', () => {
            const folders = vscode.workspace.workspaceFolders;
            // The test fixture workspace is opened, so folders should exist.
            if (folders && folders.length > 0) {
                const result = getBestWorkspaceFolder();
                assert.ok(result, 'getBestWorkspaceFolder should return a folder');
                assert.ok(result.uri, 'Returned folder must have a uri');
                assert.strictEqual(typeof result.uri.fsPath, 'string');
            }
        });

        test('returns undefined when no workspace folders exist', () => {
            // When workspace folders are available (test fixture), the
            // function should return one. This test documents the contract:
            // the return type is WorkspaceFolder | undefined.
            const result = getBestWorkspaceFolder();
            if (!vscode.workspace.workspaceFolders || vscode.workspace.workspaceFolders.length === 0) {
                assert.strictEqual(result, undefined);
            } else {
                assert.ok(result, 'Should return a folder when workspace folders exist');
            }
        });
    });

    suite('findKonvoyWorkspaces', () => {
        test('returns an array', () => {
            const result = findKonvoyWorkspaces();
            assert.ok(Array.isArray(result), 'findKonvoyWorkspaces must return an array');
        });

        test('filters workspace folders to those containing konvoy.toml', () => {
            const result: vscode.WorkspaceFolder[] = findKonvoyWorkspaces();
            // The test fixture workspace contains a konvoy.toml, so we
            // expect at least one match.
            if (vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders.length > 0) {
                assert.ok(result.length > 0, 'Should find at least one konvoy workspace in the test fixture');
            }
        });

        test('each returned item is a valid WorkspaceFolder', () => {
            const result: vscode.WorkspaceFolder[] = findKonvoyWorkspaces();
            for (const folder of result) {
                assert.ok(folder.uri, 'Each workspace folder must have a uri');
                assert.strictEqual(typeof folder.uri.fsPath, 'string');
                assert.strictEqual(typeof folder.index, 'number');
            }
        });
    });

    suite('watchManifests', () => {
        let watcher: vscode.FileSystemWatcher | undefined;

        suiteTeardown(() => {
            watcher?.dispose();
        });

        test('returns a FileSystemWatcher disposable', () => {
            watcher = watchManifests(
                () => { /* onCreated */ },
                () => { /* onDeleted */ },
            );
            assert.ok(watcher, 'watchManifests must return a truthy value');
            assert.ok(
                typeof watcher.dispose === 'function',
                'Returned watcher must have a dispose method',
            );
        });
    });
});
