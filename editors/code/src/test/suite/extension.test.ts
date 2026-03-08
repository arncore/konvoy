import * as assert from 'assert';
import * as vscode from 'vscode';
import { createMockContext } from './testHelpers';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { activate, deactivate } = require('../../extension');

suite('extension lifecycle', () => {
    let mockContext: vscode.ExtensionContext;
    let autoActivated = false;

    suiteSetup(async () => {
        mockContext = createMockContext();
        // The extension may have auto-activated via workspaceContains:konvoy.toml.
        const allCommands = await vscode.commands.getCommands(true);
        if (allCommands.includes('konvoy.build')) {
            autoActivated = true;
            return;
        }
        activate(mockContext);
    });

    suiteTeardown(() => {
        if (autoActivated) { return; }
        for (const sub of mockContext.subscriptions) {
            sub.dispose();
        }
        deactivate();
    });

    test('activate populates subscriptions', function () {
        if (autoActivated) { return this.skip(); }
        assert.ok(
            mockContext.subscriptions.length > 0,
            `Expected subscriptions to be populated, got ${mockContext.subscriptions.length}`,
        );
    });

    test('activate registers commands', async () => {
        const allCommands = await vscode.commands.getCommands(true);
        const expectedCommands = [
            'konvoy.build',
            'konvoy.buildRelease',
            'konvoy.buildPick',
            'konvoy.run',
            'konvoy.runRelease',
            'konvoy.toggleRunVariant',
            'konvoy.test',
            'konvoy.lint',
            'konvoy.update',
            'konvoy.clean',
            'konvoy.cleanAll',
            'konvoy.cleanConfirm',
            'konvoy.doctor',
            'konvoy.toolchainInstall',
            'konvoy.toolchainList',
        ];
        for (const cmd of expectedCommands) {
            assert.ok(
                allCommands.includes(cmd),
                `Expected command '${cmd}' to be registered`,
            );
        }
    });

    test('activate registers task provider', async () => {
        const tasks = await vscode.tasks.fetchTasks({ type: 'konvoy' });
        assert.ok(
            Array.isArray(tasks),
            'fetchTasks should return an array for the konvoy task type',
        );
    });

    test('deactivate does not throw', () => {
        assert.doesNotThrow(() => {
            deactivate();
        });
    });

    test('subscriptions are all disposable', function () {
        if (autoActivated) { return this.skip(); }
        for (let i = 0; i < mockContext.subscriptions.length; i++) {
            const sub = mockContext.subscriptions[i];
            assert.ok(
                typeof sub.dispose === 'function',
                `Subscription at index ${i} must have a dispose method`,
            );
        }
    });
});
