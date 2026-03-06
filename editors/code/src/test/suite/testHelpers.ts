import * as vscode from 'vscode';

/**
 * Creates a minimal mock of vscode.ExtensionContext sufficient for
 * testing without a real extension host. Uses a Map-backed
 * workspaceState so get/update calls work as expected.
 */
export function createMockContext(): vscode.ExtensionContext {
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
