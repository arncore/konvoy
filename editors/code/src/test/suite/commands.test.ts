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

    test('executing konvoy.build does not throw', async () => {
        // In the test environment there is no workspace folder with
        // konvoy.toml, so the command should show an error message but
        // must never throw an unhandled exception.
        try {
            await vscode.commands.executeCommand('konvoy.build');
        } catch (err) {
            assert.fail(`konvoy.build threw an unexpected error: ${err}`);
        }
    });

    test('COMMANDS array entries have expected structure', () => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { COMMANDS } = require('../../commands');
        assert.ok(Array.isArray(COMMANDS), 'COMMANDS should be an array');
        assert.ok(COMMANDS.length > 0, 'COMMANDS should not be empty');

        for (const cmd of COMMANDS) {
            assert.ok(
                typeof cmd.id === 'string' && cmd.id.length > 0,
                `Command must have a non-empty string id, got: ${JSON.stringify(cmd.id)}`,
            );
            assert.ok(
                Array.isArray(cmd.args),
                `Command "${cmd.id}" must have an args array`,
            );
            assert.strictEqual(
                typeof cmd.parseDiagnostics,
                'boolean',
                `Command "${cmd.id}" must have a boolean parseDiagnostics`,
            );
            assert.strictEqual(
                typeof cmd.useDetektParser,
                'boolean',
                `Command "${cmd.id}" must have a boolean useDetektParser`,
            );
        }
    });

    test('COMMANDS ids match EXPECTED_COMMAND_IDS', () => {
        // eslint-disable-next-line @typescript-eslint/no-var-requires
        const { COMMANDS } = require('../../commands');
        const ids = COMMANDS.map((c: { id: string }) => c.id);
        for (const expectedId of EXPECTED_COMMAND_IDS) {
            assert.ok(
                ids.includes(expectedId),
                `Expected COMMANDS to contain "${expectedId}"`,
            );
        }
    });
});
