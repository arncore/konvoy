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

    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const { COMMANDS, _testing } = require('../../commands');

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

    test('every EXPECTED_COMMAND_ID is returned by getCommands()', async () => {
        const allCommands = await vscode.commands.getCommands(true);
        for (const commandId of EXPECTED_COMMAND_IDS) {
            assert.ok(
                allCommands.includes(commandId),
                `Expected command "${commandId}" to be registered`,
            );
        }
    });

    test('executing command without workspace does not throw', async () => {
        // In the test environment there is no workspace folder with
        // konvoy.toml, so the command should show an error message but
        // must never throw an unhandled exception.
        try {
            await vscode.commands.executeCommand('konvoy.build');
        } catch (err) {
            assert.fail(`konvoy.build threw an unexpected error: ${err}`);
        }
    });

    // --- runCommand guard tests using _testing helper ---

    test('shows warning when a command is already running', async () => {
        _testing.setRunning({} as any);
        try {
            // Should not throw; internally it shows a warning and returns early.
            await vscode.commands.executeCommand('konvoy.build');
        } catch (err) {
            assert.fail(`konvoy.build threw while another process was running: ${err}`);
        } finally {
            _testing.setRunning(undefined);
        }
    });

    test('resets running state on process error', async () => {
        // In CI the konvoy binary does not exist, so spawn emits ENOENT.
        // After the error handler fires, runningProcess must be cleared.
        assert.strictEqual(_testing.isRunning(), false, 'precondition: nothing running');

        await vscode.commands.executeCommand('konvoy.build');

        // The error event is asynchronous; give the event loop a tick.
        await new Promise(resolve => setTimeout(resolve, 200));

        assert.strictEqual(
            _testing.isRunning(),
            false,
            'runningProcess should be cleared after a process error',
        );
    });

    // --- COMMANDS structure tests ---

    test('all command IDs start with konvoy.', () => {
        for (const cmd of COMMANDS) {
            assert.ok(
                cmd.id.startsWith('konvoy.'),
                `Command id "${cmd.id}" does not start with "konvoy."`,
            );
        }
    });

    test('all commands have non-empty args array', () => {
        for (const cmd of COMMANDS) {
            assert.ok(
                Array.isArray(cmd.args) && cmd.args.length > 0,
                `Command "${cmd.id}" must have a non-empty args array`,
            );
        }
    });

    test('commands with parseDiagnostics have valid useDetektParser boolean', () => {
        for (const cmd of COMMANDS) {
            if (cmd.parseDiagnostics) {
                assert.strictEqual(
                    typeof cmd.useDetektParser,
                    'boolean',
                    `Command "${cmd.id}" has parseDiagnostics: true but useDetektParser is not a boolean`,
                );
            }
        }
    });

    test('konvoy.lint is the only command with useDetektParser: true', () => {
        const detektCommands = COMMANDS.filter(
            (c: { useDetektParser: boolean }) => c.useDetektParser === true,
        );
        assert.strictEqual(
            detektCommands.length,
            1,
            `Expected exactly 1 command with useDetektParser: true, got ${detektCommands.length}`,
        );
        assert.strictEqual(
            detektCommands[0].id,
            'konvoy.lint',
            `Expected konvoy.lint to be the only useDetektParser command, got "${detektCommands[0].id}"`,
        );
    });
});
