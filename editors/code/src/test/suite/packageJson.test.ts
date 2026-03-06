import * as assert from 'assert';
import * as path from 'path';
import * as fs from 'fs';

/**
 * Validates the VS Code extension package.json manifest.
 *
 * These tests ensure that all commands have the expected icons, menu
 * contributions reference the correct file types (including konvoy.lock),
 * and navigation groups are ordered correctly.
 */

interface PackageJsonCommand {
    command: string;
    title: string;
    icon?: string;
}

interface PackageJsonMenuItem {
    command: string;
    group: string;
    when: string;
}

interface PackageJson {
    contributes: {
        commands: PackageJsonCommand[];
        menus: {
            'editor/title': PackageJsonMenuItem[];
            'editor/title/run': PackageJsonMenuItem[];
        };
    };
}

function loadPackageJson(): PackageJson {
    const packageJsonPath = path.resolve(__dirname, '..', '..', '..', 'package.json');
    const raw = fs.readFileSync(packageJsonPath, 'utf-8');
    return JSON.parse(raw) as PackageJson;
}

suite('package.json manifest validation', () => {
    let pkg: PackageJson;

    suiteSetup(() => {
        pkg = loadPackageJson();
    });

    // --- Command icon tests ---

    /**
     * Commands that are intentionally excluded from icon requirements:
     * - konvoy.toolchainInstall: palette-only, not shown in editor title
     * - konvoy.toolchainList: palette-only, not shown in editor title
     * - konvoy.cleanAll: hidden command invoked by cleanConfirm, not shown directly
     */
    const COMMANDS_WITHOUT_ICONS = [
        'konvoy.toolchainInstall',
        'konvoy.toolchainList',
        'konvoy.cleanAll',
        'konvoy.toggleRunVariant',
    ];

    test('all commands except toolchain and cleanAll have icons', () => {
        const commands = pkg.contributes.commands;
        for (const cmd of commands) {
            if (COMMANDS_WITHOUT_ICONS.includes(cmd.command)) {
                continue;
            }
            assert.ok(
                cmd.icon !== undefined && cmd.icon !== '',
                `Command "${cmd.command}" should have an icon defined but does not`,
            );
        }
    });

    test('toolchain commands and cleanAll do not have icons', () => {
        const commands = pkg.contributes.commands;
        for (const cmdId of COMMANDS_WITHOUT_ICONS) {
            const cmd = commands.find(c => c.command === cmdId);
            assert.ok(cmd, `Command "${cmdId}" must exist in package.json`);
            assert.strictEqual(
                cmd!.icon,
                undefined,
                `Command "${cmdId}" should not have an icon`,
            );
        }
    });

    test('konvoy.build has $(tools) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.build');
        assert.ok(cmd, 'konvoy.build must exist');
        assert.strictEqual(cmd!.icon, '$(tools)');
    });

    test('konvoy.buildRelease has $(tools) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.buildRelease');
        assert.ok(cmd, 'konvoy.buildRelease must exist');
        assert.strictEqual(cmd!.icon, '$(tools)');
    });

    test('konvoy.test has $(beaker) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.test');
        assert.ok(cmd, 'konvoy.test must exist');
        assert.strictEqual(cmd!.icon, '$(beaker)');
    });

    test('konvoy.lint has $(eye) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.lint');
        assert.ok(cmd, 'konvoy.lint must exist');
        assert.strictEqual(cmd!.icon, '$(eye)');
    });

    test('konvoy.doctor has $(pulse) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.doctor');
        assert.ok(cmd, 'konvoy.doctor must exist');
        assert.strictEqual(cmd!.icon, '$(pulse)');
    });

    test('konvoy.run has $(debug-alt) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.run');
        assert.ok(cmd, 'konvoy.run must exist');
        assert.strictEqual(cmd!.icon, '$(debug-alt)');
    });

    test('konvoy.runRelease has $(play) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.runRelease');
        assert.ok(cmd, 'konvoy.runRelease must exist');
        assert.strictEqual(cmd!.icon, '$(play)');
    });

    test('konvoy.buildPick has $(tools) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.buildPick');
        assert.ok(cmd, 'konvoy.buildPick must exist');
        assert.strictEqual(cmd!.icon, '$(tools)');
    });

    test('konvoy.update has $(sync) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.update');
        assert.ok(cmd, 'konvoy.update must exist');
        assert.strictEqual(cmd!.icon, '$(sync)');
    });

    test('konvoy.cleanConfirm has $(eraser) icon', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.cleanConfirm');
        assert.ok(cmd, 'konvoy.cleanConfirm must exist');
        assert.strictEqual(cmd!.icon, '$(eraser)');
    });

    // --- editor/title menu tests ---

    const EXPECTED_WHEN_CLAUSE = "resourceFilename == 'konvoy.toml' || resourceFilename == 'konvoy.lock' || resourceExtname == '.kt'";

    test('editor/title has exactly 6 menu entries', () => {
        const entries = pkg.contributes.menus['editor/title'];
        assert.strictEqual(
            entries.length,
            6,
            `Expected 6 editor/title entries, got ${entries.length}`,
        );
    });

    test('all editor/title entries include konvoy.lock in when clause', () => {
        const entries = pkg.contributes.menus['editor/title'];
        for (const entry of entries) {
            assert.ok(
                entry.when.includes("resourceFilename == 'konvoy.lock'"),
                `Menu entry for "${entry.command}" when clause must include konvoy.lock, got: "${entry.when}"`,
            );
        }
    });

    test('all editor/title entries include konvoy.toml in when clause', () => {
        const entries = pkg.contributes.menus['editor/title'];
        for (const entry of entries) {
            assert.ok(
                entry.when.includes("resourceFilename == 'konvoy.toml'"),
                `Menu entry for "${entry.command}" when clause must include konvoy.toml, got: "${entry.when}"`,
            );
        }
    });

    test('all editor/title entries include .kt files in when clause', () => {
        const entries = pkg.contributes.menus['editor/title'];
        for (const entry of entries) {
            assert.ok(
                entry.when.includes("resourceExtname == '.kt'"),
                `Menu entry for "${entry.command}" when clause must include .kt extension, got: "${entry.when}"`,
            );
        }
    });

    test('all editor/title entries have the exact expected when clause', () => {
        const entries = pkg.contributes.menus['editor/title'];
        for (const entry of entries) {
            assert.strictEqual(
                entry.when,
                EXPECTED_WHEN_CLAUSE,
                `Menu entry for "${entry.command}" has unexpected when clause`,
            );
        }
    });

    test('editor/title navigation groups are ordered 1 through 6', () => {
        const entries = pkg.contributes.menus['editor/title'];
        for (let i = 0; i < entries.length; i++) {
            assert.strictEqual(
                entries[i].group,
                `navigation@${i + 1}`,
                `Entry at index ${i} for "${entries[i].command}" should have group "navigation@${i + 1}", got "${entries[i].group}"`,
            );
        }
    });

    test('editor/title button order is BuildPick, Test, Update, Lint, CleanConfirm, Doctor', () => {
        const entries = pkg.contributes.menus['editor/title'];
        const expectedOrder = [
            'konvoy.buildPick',
            'konvoy.test',
            'konvoy.update',
            'konvoy.lint',
            'konvoy.cleanConfirm',
            'konvoy.doctor',
        ];
        const actualOrder = entries.map(e => e.command);
        assert.deepStrictEqual(
            actualOrder,
            expectedOrder,
            `Editor title button order mismatch`,
        );
    });

    // --- editor/title/run menu tests ---

    test('editor/title/run has exactly 2 menu entries', () => {
        const entries = pkg.contributes.menus['editor/title/run'];
        assert.strictEqual(
            entries.length,
            2,
            `Expected 2 editor/title/run entries, got ${entries.length}`,
        );
    });

    test('all editor/title/run entries include konvoy.lock in when clause', () => {
        const entries = pkg.contributes.menus['editor/title/run'];
        for (const entry of entries) {
            assert.ok(
                entry.when.includes("resourceFilename == 'konvoy.lock'"),
                `Run menu entry for "${entry.command}" when clause must include konvoy.lock, got: "${entry.when}"`,
            );
        }
    });

    test('editor/title/run entries have profile-aware when clauses', () => {
        const entries = pkg.contributes.menus['editor/title/run'];
        const runEntry = entries.find(e => e.command === 'konvoy.run');
        const runReleaseEntry = entries.find(e => e.command === 'konvoy.runRelease');
        assert.ok(runEntry, 'konvoy.run must be in editor/title/run');
        assert.ok(runReleaseEntry, 'konvoy.runRelease must be in editor/title/run');
        assert.ok(
            runEntry!.when.includes('!konvoy.releaseMode'),
            `konvoy.run when clause must include !konvoy.releaseMode, got: "${runEntry!.when}"`,
        );
        assert.ok(
            runReleaseEntry!.when.includes('konvoy.releaseMode') && !runReleaseEntry!.when.includes('!konvoy.releaseMode'),
            `konvoy.runRelease when clause must include konvoy.releaseMode (without negation), got: "${runReleaseEntry!.when}"`,
        );
    });

    test('editor/title/run entries both use navigation@1 for swapping in place', () => {
        const entries = pkg.contributes.menus['editor/title/run'];
        for (const entry of entries) {
            assert.strictEqual(
                entry.group,
                'navigation@1',
                `Run menu entry "${entry.command}" should use navigation@1 for in-place toggle, got "${entry.group}"`,
            );
        }
    });

    // --- Command existence and title tests ---

    test('konvoy.toggleRunVariant command exists in package.json', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.toggleRunVariant');
        assert.ok(cmd, 'konvoy.toggleRunVariant must exist in package.json commands array');
    });

    test('konvoy.toggleRunVariant has correct title', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.toggleRunVariant');
        assert.ok(cmd, 'konvoy.toggleRunVariant must exist');
        assert.strictEqual(
            cmd!.title,
            'Konvoy: Toggle Debug/Release',
            `Expected title "Konvoy: Toggle Debug/Release", got "${cmd!.title}"`,
        );
    });

    test('konvoy.buildPick has correct title "Konvoy: Build..."', () => {
        const cmd = pkg.contributes.commands.find(c => c.command === 'konvoy.buildPick');
        assert.ok(cmd, 'konvoy.buildPick must exist');
        assert.strictEqual(
            cmd!.title,
            'Konvoy: Build...',
            `Expected title "Konvoy: Build...", got "${cmd!.title}"`,
        );
    });

    // --- Cross-validation: editor/title commands must exist in commands array ---

    test('every editor/title menu command exists in commands array', () => {
        const commandIds = pkg.contributes.commands.map(c => c.command);
        const menuEntries = pkg.contributes.menus['editor/title'];
        for (const entry of menuEntries) {
            assert.ok(
                commandIds.includes(entry.command),
                `Menu entry references "${entry.command}" but it is not defined in commands array`,
            );
        }
    });

    test('every editor/title/run menu command exists in commands array', () => {
        const commandIds = pkg.contributes.commands.map(c => c.command);
        const menuEntries = pkg.contributes.menus['editor/title/run'];
        for (const entry of menuEntries) {
            assert.ok(
                commandIds.includes(entry.command),
                `Run menu entry references "${entry.command}" but it is not defined in commands array`,
            );
        }
    });

    // --- Ensure all menu-visible commands have icons ---

    test('every command in editor/title menu has an icon', () => {
        const entries = pkg.contributes.menus['editor/title'];
        for (const entry of entries) {
            const cmd = pkg.contributes.commands.find(c => c.command === entry.command);
            assert.ok(cmd, `Command "${entry.command}" must exist`);
            assert.ok(
                cmd!.icon !== undefined && cmd!.icon !== '',
                `Command "${entry.command}" appears in editor/title menu but has no icon`,
            );
        }
    });

    test('every command in editor/title/run menu has an icon', () => {
        const entries = pkg.contributes.menus['editor/title/run'];
        for (const entry of entries) {
            const cmd = pkg.contributes.commands.find(c => c.command === entry.command);
            assert.ok(cmd, `Command "${entry.command}" must exist`);
            assert.ok(
                cmd!.icon !== undefined && cmd!.icon !== '',
                `Command "${entry.command}" appears in editor/title/run menu but has no icon`,
            );
        }
    });
});
