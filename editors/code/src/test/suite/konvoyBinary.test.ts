import * as assert from 'assert';
import * as vscode from 'vscode';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { getKonvoyPath } = require('../../konvoyBinary');

suite('konvoyBinary', () => {
    suite('getKonvoyPath', () => {
        test('returns "konvoy" when no config is set (default fallback)', () => {
            // Clear any existing configuration override so we get the default.
            const config = vscode.workspace.getConfiguration('konvoy');
            const currentValue = config.get<string>('path', '');
            // When no user/workspace setting is configured the default is
            // an empty string, so getKonvoyPath should fall back to 'konvoy'.
            if (!currentValue) {
                const result = getKonvoyPath();
                assert.strictEqual(
                    result,
                    'konvoy',
                    'getKonvoyPath should return "konvoy" when config is unset',
                );
            }
        });

        test('returns a non-empty string', () => {
            const result = getKonvoyPath();
            assert.strictEqual(typeof result, 'string', 'getKonvoyPath must return a string');
            assert.ok(result.length > 0, 'getKonvoyPath must return a non-empty string');
        });
    });
});
