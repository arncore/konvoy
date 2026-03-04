import * as assert from 'assert';
import * as vscode from 'vscode';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { getKonvoyPath } = require('../../konvoyBinary');

suite('konvoyBinary', () => {
    suite('getKonvoyPath', () => {
        test('returns default "konvoy" when config is empty', () => {
            const config = vscode.workspace.getConfiguration('konvoy');
            const configuredPath = config.get<string>('path', '');
            const result = getKonvoyPath();
            assert.ok(result.length > 0, 'Must return a non-empty path');
            if (!configuredPath) {
                assert.strictEqual(result, 'konvoy', 'Default should be "konvoy"');
            }
        });

        test('returns custom path when konvoy.path is configured', async () => {
            const config = vscode.workspace.getConfiguration('konvoy');
            const original = config.get<string>('path', '');
            try {
                await config.update('path', '/custom/konvoy', vscode.ConfigurationTarget.Global);
                const result = getKonvoyPath();
                assert.strictEqual(result, '/custom/konvoy');
            } finally {
                await config.update('path', original || undefined, vscode.ConfigurationTarget.Global);
            }
        });

        test('returns a non-empty string', () => {
            const result = getKonvoyPath();
            assert.strictEqual(typeof result, 'string', 'getKonvoyPath must return a string');
            assert.ok(result.length > 0, 'getKonvoyPath must return a non-empty string');
        });
    });
});
