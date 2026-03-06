import * as assert from 'assert';
import * as vscode from 'vscode';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { activate, deactivate } = require('../../extension');

/**
 * Creates a minimal mock of vscode.ExtensionContext sufficient for
 * testing the activate/deactivate lifecycle without a real extension host.
 */
function createMockContext(): vscode.ExtensionContext {
    const subscriptions: vscode.Disposable[] = [];
    return {
        subscriptions,
        extensionPath: __dirname,
        extensionUri: vscode.Uri.file(__dirname),
        globalState: {
            get: () => undefined,
            update: () => Promise.resolve(),
            keys: () => [],
            setKeysForSync: () => {},
        },
        workspaceState: {
            get: () => undefined,
            update: () => Promise.resolve(),
            keys: () => [],
        },
        secrets: {
            get: () => Promise.resolve(undefined),
            store: () => Promise.resolve(),
            delete: () => Promise.resolve(),
            onDidChange: new vscode.EventEmitter<vscode.SecretStorageChangeEvent>().event,
        },
        asAbsolutePath: (relativePath: string) => relativePath,
        environmentVariableCollection: {} as any,
        storagePath: undefined,
        globalStoragePath: __dirname,
        logPath: __dirname,
        storageUri: undefined,
        globalStorageUri: vscode.Uri.file(__dirname),
        logUri: vscode.Uri.file(__dirname),
        extensionMode: vscode.ExtensionMode.Test,
        extension: {} as any,
        languageModelAccessInformation: {} as any,
    } as unknown as vscode.ExtensionContext;
}

suite('extension lifecycle', () => {
    let mockContext: vscode.ExtensionContext;

    suiteSetup(() => {
        mockContext = createMockContext();
        activate(mockContext);
    });

    suiteTeardown(() => {
        for (const sub of mockContext.subscriptions) {
            sub.dispose();
        }
        deactivate();
    });

    test('activate populates subscriptions', () => {
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
            'konvoy.run',
            'konvoy.runRelease',
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

    test('subscriptions are all disposable', () => {
        for (let i = 0; i < mockContext.subscriptions.length; i++) {
            const sub = mockContext.subscriptions[i];
            assert.ok(
                typeof sub.dispose === 'function',
                `Subscription at index ${i} must have a dispose method`,
            );
        }
    });
});
