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

    test('registerCommands returns 11 disposables', () => {
        // Import and call registerCommands directly to verify it returns
        // the correct number of disposables. We dispose them immediately
        // to avoid duplicate registrations.
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { registerCommands } = require('../../commands');
        const disposables: vscode.Disposable[] = registerCommands();
        try {
            assert.strictEqual(
                disposables.length,
                11,
                `Expected 11 disposables, got ${disposables.length}`,
            );
            for (const d of disposables) {
                assert.ok(d.dispose, 'Each disposable must have a dispose method');
            }
        } finally {
            // Clean up to avoid duplicate command registrations
            for (const d of disposables) {
                d.dispose();
            }
        }
    });
});
