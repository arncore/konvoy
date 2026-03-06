import * as assert from 'assert';
import * as vscode from 'vscode';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { initProfile, _testing } = require('../../profileManager');

/**
 * Creates a minimal mock of vscode.ExtensionContext sufficient for
 * testing initProfile without a real extension host.
 */
function createMockContext(): vscode.ExtensionContext {
    const store = new Map<string, unknown>();
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
            get: (key: string) => store.get(key),
            update: (key: string, value: unknown) => {
                store.set(key, value);
                return Promise.resolve();
            },
            keys: () => [...store.keys()],
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

suite('profileManager', () => {
    let disposables: vscode.Disposable[] = [];
    let mockContext: vscode.ExtensionContext;

    suiteSetup(() => {
        _testing.resetState();
        mockContext = createMockContext();
        disposables = initProfile(mockContext);
    });

    suiteTeardown(() => {
        for (const d of disposables) {
            d.dispose();
        }
        _testing.resetState();
    });

    // --- _testing helpers ---

    test('_testing helpers exist and are callable', () => {
        assert.strictEqual(typeof _testing.getStatusBarItem, 'function', '_testing.getStatusBarItem must be a function');
        assert.strictEqual(typeof _testing.resetState, 'function', '_testing.resetState must be a function');
    });

    // --- Default profile ---

    test('default profile is debug', () => {
        assert.strictEqual(
            _testing.getRunProfile(),
            'debug',
            'Default run profile should be "debug"',
        );
    });

    test('getRunProfile returns debug or release', () => {
        const profile = _testing.getRunProfile();
        assert.ok(
            profile === 'debug' || profile === 'release',
            `getRunProfile() must return "debug" or "release", got "${profile}"`,
        );
    });

    // --- initProfile ---

    test('initProfile returns a disposables array', () => {
        assert.ok(Array.isArray(disposables), 'initProfile must return an array');
        assert.ok(disposables.length > 0, 'initProfile must return at least one disposable');
        for (const d of disposables) {
            assert.ok(typeof d.dispose === 'function', 'Each item must have a dispose method');
        }
    });

    // --- Status bar item ---

    test('initProfile creates a status bar item', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must be created after initProfile');
    });

    test('status bar item has correct initial text "$(debug-alt) Debug"', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must exist');
        assert.strictEqual(
            item.text,
            '$(debug-alt) Debug',
            `Expected status bar text "$(debug-alt) Debug", got "${item.text}"`,
        );
    });

    test('status bar item command is konvoy.toggleRunProfile', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must exist');
        assert.strictEqual(
            item.command,
            'konvoy.toggleRunProfile',
            `Expected command "konvoy.toggleRunProfile", got "${item.command}"`,
        );
    });

    test('status bar item has a tooltip', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must exist');
        assert.ok(
            item.tooltip && (item.tooltip as string).length > 0,
            'Status bar item should have a non-empty tooltip',
        );
    });

    // --- Toggle behavior ---

    test('toggle switches profile from debug to release', async () => {
        assert.strictEqual(_testing.getRunProfile(), 'debug', 'precondition: profile is debug');

        await vscode.commands.executeCommand('konvoy.toggleRunProfile');

        assert.strictEqual(
            _testing.getRunProfile(),
            'release',
            'Profile should be "release" after toggling from debug',
        );
    });

    test('status bar text updates to release after toggle', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must exist');
        assert.strictEqual(
            item.text,
            '$(play) Release',
            `Expected status bar text "$(play) Release", got "${item.text}"`,
        );
    });

    test('toggle switches profile back from release to debug', async () => {
        assert.strictEqual(_testing.getRunProfile(), 'release', 'precondition: profile is release');

        await vscode.commands.executeCommand('konvoy.toggleRunProfile');

        assert.strictEqual(
            _testing.getRunProfile(),
            'debug',
            'Profile should be "debug" after toggling from release',
        );
    });

    test('status bar text updates back to debug after second toggle', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must exist');
        assert.strictEqual(
            item.text,
            '$(debug-alt) Debug',
            `Expected status bar text "$(debug-alt) Debug", got "${item.text}"`,
        );
    });

    test('workspaceState persists the toggled profile', async () => {
        await vscode.commands.executeCommand('konvoy.toggleRunProfile');
        const stored = mockContext.workspaceState.get('konvoy.runProfile');
        assert.strictEqual(
            stored,
            'release',
            `Expected workspaceState to store "release", got "${stored}"`,
        );
        // Toggle back to debug for clean state
        await vscode.commands.executeCommand('konvoy.toggleRunProfile');
    });
});
