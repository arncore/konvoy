import * as assert from 'assert';
import * as vscode from 'vscode';

const EXPECTED_COMMAND_IDS = [
    'konvoy.build',
    'konvoy.buildRelease',
    'konvoy.run',
    'konvoy.runRelease',
    'konvoy.test',
    'konvoy.lint',
    'konvoy.update',
    'konvoy.clean',
    'konvoy.doctor',
    'konvoy.toolchainInstall',
    'konvoy.toolchainList',
];

suite('Commands', () => {
    // Register commands directly so tests don't depend on extension
    // activation (which requires extensionDependencies like fwcd.kotlin
    // that aren't available in CI).
    let disposables: vscode.Disposable[] = [];

    suiteSetup(() => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { registerCommands } = require('../../commands');
        disposables = registerCommands();
    });

    suiteTeardown(() => {
        for (const d of disposables) {
            d.dispose();
        }
    });

    test('registerCommands returns 11 disposables', () => {
        assert.strictEqual(
            disposables.length,
            11,
            `Expected 11 disposables, got ${disposables.length}`,
        );
        for (const d of disposables) {
            assert.ok(d.dispose, 'Each disposable must have a dispose method');
        }
    });

    test('all 11 konvoy commands are registered', async () => {
        const allCommands = await vscode.commands.getCommands(true);
        const konvoyCommands = allCommands.filter(id => id.startsWith('konvoy.'));
        assert.strictEqual(
            konvoyCommands.length,
            11,
            `Expected 11 konvoy commands, got ${konvoyCommands.length}: ${JSON.stringify(konvoyCommands)}`,
        );
    });

    for (const commandId of EXPECTED_COMMAND_IDS) {
        test(`command "${commandId}" is registered`, async () => {
            const allCommands = await vscode.commands.getCommands(true);
            assert.ok(
                allCommands.includes(commandId),
                `Expected command "${commandId}" to be registered`,
            );
        });
    }
});
