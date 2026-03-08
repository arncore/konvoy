import * as assert from 'assert';
import * as vscode from 'vscode';

suite('TaskProvider', () => {
    // Register the task provider directly for isolated testing.
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

    test('resolveTask returns undefined for an unknown command', () => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { KonvoyTaskProvider } = require('../../taskProvider');
        const provider = new KonvoyTaskProvider();

        // Create a task whose definition has an unrecognized command.
        // resolveTask should return undefined when no matching config is found.
        const unknownDefinition = { type: 'konvoy', command: 'nonexistent' };
        const dummyTask = new vscode.Task(
            unknownDefinition,
            vscode.TaskScope.Workspace,
            'dummy',
            'konvoy',
        );
        const result = provider.resolveTask(dummyTask);
        assert.strictEqual(result, undefined, 'resolveTask should return undefined for an unknown command');
    });

    test('resolveTask returns a task for a known command', () => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { KonvoyTaskProvider } = require('../../taskProvider');
        const provider = new KonvoyTaskProvider();

        const buildDefinition = { type: 'konvoy', command: 'build' };
        const buildTask = new vscode.Task(
            buildDefinition,
            vscode.TaskScope.Workspace,
            'build',
            'konvoy',
        );
        const resolved = provider.resolveTask(buildTask);
        assert.ok(resolved, 'resolveTask should return a Task for a known command');
        assert.strictEqual(resolved.definition.type, 'konvoy');
    });

    test('provideTasks returns an array of tasks', () => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { KonvoyTaskProvider } = require('../../taskProvider');
        const provider = new KonvoyTaskProvider();

        const tasks = provider.provideTasks();
        assert.ok(Array.isArray(tasks), 'provideTasks should return an array');
        assert.ok(tasks.length > 0, 'provideTasks should return at least one task');

        for (const task of tasks) {
            assert.strictEqual(
                task.definition.type,
                'konvoy',
                'Every provided task should have type "konvoy"',
            );
        }
    });

    test('invalidate causes provideTasks to rebuild the task list', () => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { KonvoyTaskProvider } = require('../../taskProvider');
        const provider = new KonvoyTaskProvider();

        // First call populates the cached task list.
        const firstTasks = provider.provideTasks();
        assert.ok(Array.isArray(firstTasks), 'First provideTasks call should return an array');

        // Second call without invalidate returns the same cached array reference.
        const cachedTasks = provider.provideTasks();
        assert.strictEqual(
            firstTasks,
            cachedTasks,
            'Without invalidate, provideTasks should return the same array reference (cached)',
        );

        // After invalidating, provideTasks should rebuild and return a new array.
        provider.invalidate();
        const rebuiltTasks = provider.provideTasks();
        assert.ok(Array.isArray(rebuiltTasks), 'Rebuilt tasks should be an array');
        assert.ok(rebuiltTasks.length > 0, 'Rebuilt tasks should not be empty');
        assert.notStrictEqual(
            firstTasks,
            rebuiltTasks,
            'After invalidate, provideTasks should return a new array (not the cached reference)',
        );
    });
});
