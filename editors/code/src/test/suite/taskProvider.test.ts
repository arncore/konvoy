import * as assert from 'assert';
import * as vscode from 'vscode';

suite('TaskProvider', () => {
    suiteSetup(async () => {
        const ext = vscode.extensions.getExtension('konvoy.konvoy-vscode');
        if (ext && !ext.isActive) {
            await ext.activate();
        }
    });

    test('fetching konvoy tasks does not throw', async () => {
        // fetchTasks may return an empty array if no konvoy.toml exists in
        // the test workspace, but it must not throw.
        const tasks = await vscode.tasks.fetchTasks({ type: 'konvoy' });
        assert.ok(Array.isArray(tasks), 'fetchTasks should return an array');
    });

    test('registerTaskProvider returns a Disposable', () => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { registerTaskProvider } = require('../../taskProvider');
        const disposable: vscode.Disposable = registerTaskProvider();
        try {
            assert.ok(disposable, 'registerTaskProvider must return a truthy value');
            assert.ok(
                typeof disposable.dispose === 'function',
                'Returned value must have a dispose method',
            );
        } finally {
            disposable.dispose();
        }
    });

    test('all provided tasks have type konvoy', async () => {
        const tasks = await vscode.tasks.fetchTasks({ type: 'konvoy' });
        for (const task of tasks) {
            assert.strictEqual(
                task.definition.type,
                'konvoy',
                `Expected task type "konvoy", got "${task.definition.type}"`,
            );
        }
    });

    test('all provided tasks have source konvoy', async () => {
        const tasks = await vscode.tasks.fetchTasks({ type: 'konvoy' });
        for (const task of tasks) {
            assert.strictEqual(
                task.source,
                'konvoy',
                `Expected task source "konvoy", got "${task.source}"`,
            );
        }
    });
});
