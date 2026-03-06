import * as assert from 'assert';
import * as vscode from 'vscode';
import { createMockContext } from './testHelpers';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { initVariant, _testing } = require('../../variantManager');

suite('variantManager', () => {
    let disposables: vscode.Disposable[] = [];
    let mockContext: vscode.ExtensionContext;

    suiteSetup(() => {
        _testing.resetState();
        mockContext = createMockContext();
        disposables = initVariant(mockContext);
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

    // --- Default variant ---

    test('default variant is debug', () => {
        assert.strictEqual(
            _testing.getRunVariant(),
            'debug',
            'Default run variant should be "debug"',
        );
    });

    test('getRunVariant returns debug or release', () => {
        const variant = _testing.getRunVariant();
        assert.ok(
            variant === 'debug' || variant === 'release',
            `getRunVariant() must return "debug" or "release", got "${variant}"`,
        );
    });

    // --- initVariant ---

    test('initVariant returns a disposables array', () => {
        assert.ok(Array.isArray(disposables), 'initVariant must return an array');
        assert.ok(disposables.length > 0, 'initVariant must return at least one disposable');
        for (const d of disposables) {
            assert.ok(typeof d.dispose === 'function', 'Each item must have a dispose method');
        }
    });

    // --- Status bar item ---

    test('initVariant creates a status bar item', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must be created after initVariant');
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

    test('status bar item command is konvoy.toggleRunVariant', () => {
        const item = _testing.getStatusBarItem();
        assert.ok(item, 'Status bar item must exist');
        assert.strictEqual(
            item.command,
            'konvoy.toggleRunVariant',
            `Expected command "konvoy.toggleRunVariant", got "${item.command}"`,
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

    test('toggle switches variant from debug to release', async () => {
        assert.strictEqual(_testing.getRunVariant(), 'debug', 'precondition: variant is debug');

        await vscode.commands.executeCommand('konvoy.toggleRunVariant');

        assert.strictEqual(
            _testing.getRunVariant(),
            'release',
            'Variant should be "release" after toggling from debug',
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

    test('toggle switches variant back from release to debug', async () => {
        assert.strictEqual(_testing.getRunVariant(), 'release', 'precondition: variant is release');

        await vscode.commands.executeCommand('konvoy.toggleRunVariant');

        assert.strictEqual(
            _testing.getRunVariant(),
            'debug',
            'Variant should be "debug" after toggling from release',
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

    test('workspaceState persists the toggled variant', async () => {
        await vscode.commands.executeCommand('konvoy.toggleRunVariant');
        const stored = mockContext.workspaceState.get('konvoy.runVariant');
        assert.strictEqual(
            stored,
            'release',
            `Expected workspaceState to store "release", got "${stored}"`,
        );
        // Toggle back to debug for clean state
        await vscode.commands.executeCommand('konvoy.toggleRunVariant');
    });
});
