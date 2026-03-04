import * as assert from 'assert';
import * as vscode from 'vscode';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { findKonvoyWorkspaces, watchManifests, getBestWorkspaceFolder } = require('../../workspaceDetector');

suite('workspaceDetector', () => {
    suite('getBestWorkspaceFolder', () => {
        test('returns a workspace folder when workspace is open', () => {
            const folder = getBestWorkspaceFolder();
            // The test fixture workspace should be open
            assert.ok(folder, 'getBestWorkspaceFolder should return a folder (test fixture workspace is open)');
            assert.ok(folder.uri, 'Folder must have a uri');
        });
    });

    suite('findKonvoyWorkspaces', () => {
        test('returns an array', () => {
            const result = findKonvoyWorkspaces();
            assert.ok(Array.isArray(result), 'findKonvoyWorkspaces must return an array');
        });

        test('finds at least one konvoy workspace in the test fixture', () => {
            const result: vscode.WorkspaceFolder[] = findKonvoyWorkspaces();
            assert.ok(result.length > 0, 'Should find at least one konvoy workspace in the test fixture');
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
