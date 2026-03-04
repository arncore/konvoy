import * as assert from 'assert';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { getOutputChannel, disposeOutputChannel } = require('../../outputChannel');

suite('outputChannel', () => {
    suiteTeardown(() => {
        disposeOutputChannel();
    });

    suite('getOutputChannel', () => {
        test('returns an OutputChannel with expected methods', () => {
            const channel = getOutputChannel();
            assert.ok(channel, 'getOutputChannel must return a truthy value');
            assert.strictEqual(
                typeof channel.appendLine,
                'function',
                'OutputChannel must have appendLine method',
            );
            assert.strictEqual(
                typeof channel.show,
                'function',
                'OutputChannel must have show method',
            );
            assert.strictEqual(
                typeof channel.clear,
                'function',
                'OutputChannel must have clear method',
            );
            assert.strictEqual(
                typeof channel.dispose,
                'function',
                'OutputChannel must have dispose method',
            );
        });

        test('returns the same instance on subsequent calls (singleton)', () => {
            const first = getOutputChannel();
            const second = getOutputChannel();
            assert.strictEqual(
                first,
                second,
                'getOutputChannel must return the same instance on repeated calls',
            );
        });
    });

    suite('disposeOutputChannel', () => {
        test('does not throw', () => {
            assert.doesNotThrow(() => {
                disposeOutputChannel();
            }, 'disposeOutputChannel must not throw');
        });
    });
});
