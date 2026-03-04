import * as assert from 'assert';
import * as vscode from 'vscode';

suite('TaskProvider', () => {
    // Register the task provider directly so tests don't depend on
    // extension activation (which requires fwcd.kotlin in CI).
    let providerDisposable: vscode.Disposable;

    suiteSetup(() => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { registerTaskProvider } = require('../../taskProvider');
        providerDisposable = registerTaskProvider();
    });

    suiteTeardown(() => {
        providerDisposable.dispose();
    });

    test('registerTaskProvider returns a Disposable', () => {
        assert.ok(providerDisposable, 'registerTaskProvider must return a truthy value');
        assert.ok(
            typeof providerDisposable.dispose === 'function',
            'Returned value must have a dispose method',
        );
    });

    test('fetching konvoy tasks does not throw', async () => {
        // fetchTasks may return an empty array if no konvoy.toml exists in
        // the test workspace, but it must not throw.
        const tasks = await vscode.tasks.fetchTasks({ type: 'konvoy' });
        assert.ok(Array.isArray(tasks), 'fetchTasks should return an array');
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
