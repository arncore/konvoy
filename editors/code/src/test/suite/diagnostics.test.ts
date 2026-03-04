import * as assert from 'assert';
import { parseKonancDiagnostics, parseDetektDiagnostics } from '../../diagnostics';

suite('parseKonancDiagnostics', () => {
    test('parses located diagnostic with file, line, and column', () => {
        const output = 'src/main.kt:10:5: error: unresolved reference: foo';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].line, 10);
        assert.strictEqual(result[0].column, 5);
        assert.strictEqual(result[0].severity, 'error');
        assert.strictEqual(result[0].message, 'unresolved reference: foo');
    });

    test('parses located diagnostic with file and line only (no column)', () => {
        const output = 'src/main.kt:10: warning: deprecated API usage';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].line, 10);
        assert.strictEqual(result[0].column, undefined);
        assert.strictEqual(result[0].severity, 'warning');
        assert.strictEqual(result[0].message, 'deprecated API usage');
    });

    test('parses bare diagnostic with no file info', () => {
        const output = 'error: could not find konanc';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, undefined);
        assert.strictEqual(result[0].line, undefined);
        assert.strictEqual(result[0].column, undefined);
        assert.strictEqual(result[0].severity, 'error');
        assert.strictEqual(result[0].message, 'could not find konanc');
    });

    test('parses multiple diagnostics from mixed output', () => {
        const output = [
            'Compilation started',
            'src/main.kt:10:5: error: unresolved reference: foo',
            'src/util.kt:3: warning: unused variable',
            'error: compilation failed',
            'Some other output line',
        ].join('\n');
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 3);
        assert.strictEqual(result[0].severity, 'error');
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[1].severity, 'warning');
        assert.strictEqual(result[1].file, 'src/util.kt');
        assert.strictEqual(result[2].severity, 'error');
        assert.strictEqual(result[2].file, undefined);
    });

    test('returns empty array for empty input', () => {
        assert.deepStrictEqual(parseKonancDiagnostics(''), []);
    });

    test('skips non-diagnostic lines', () => {
        const output = [
            'Compilation started',
            'Processing module main',
            '  linking binary...',
            'Done.',
        ].join('\n');
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 0);
    });

    test('parses info severity', () => {
        const output = 'src/main.kt:1:1: info: some informational message';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].severity, 'info');
        assert.strictEqual(result[0].message, 'some informational message');
    });

    test('parses bare warning and info', () => {
        const output = 'warning: some warning\ninfo: some info';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 2);
        assert.strictEqual(result[0].severity, 'warning');
        assert.strictEqual(result[1].severity, 'info');
    });

    test('parses paths with spaces', () => {
        const output = 'my project/src/main.kt:10:5: error: bad';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'my project/src/main.kt');
        assert.strictEqual(result[0].line, 10);
        assert.strictEqual(result[0].column, 5);
        assert.strictEqual(result[0].severity, 'error');
        assert.strictEqual(result[0].message, 'bad');
    });

    test('captures full message including colons', () => {
        const output = 'src/main.kt:10:5: error: type mismatch: expected Int, got String';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].message, 'type mismatch: expected Int, got String');
    });

    test('handles Windows \\r\\n line endings', () => {
        const output = 'src/main.kt:10:5: error: bad\r\n';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].severity, 'error');
        assert.strictEqual(result[0].message, 'bad');
    });

    test('returns empty array for whitespace-only lines', () => {
        const output = '   \n   \n   ';
        const result = parseKonancDiagnostics(output);
        assert.deepStrictEqual(result, []);
    });

    test('parses bare diagnostic with empty message', () => {
        const output = 'error: ';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].severity, 'error');
        assert.strictEqual(result[0].message, '');
    });

    test('parses diagnostics with leading whitespace on lines', () => {
        const output = '  src/main.kt:10:5: error: foo  ';
        const result = parseKonancDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].line, 10);
        assert.strictEqual(result[0].column, 5);
        assert.strictEqual(result[0].severity, 'error');
        assert.strictEqual(result[0].message, 'foo');
    });
});

suite('parseDetektDiagnostics', () => {
    test('parses real detekt format with bracketed rule', () => {
        const output = 'src/main.kt:3:5: Magic number [MagicNumber]';
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].line, 3);
        assert.strictEqual(result[0].column, 5);
        assert.strictEqual(result[0].severity, 'warning');
        assert.strictEqual(result[0].message, 'Magic number');
        assert.strictEqual(result[0].rule, 'MagicNumber');
    });

    test('parses legacy detekt format', () => {
        const output = 'src/main.kt:5:1: UnusedImport - Unused import detected [detekt.style]';
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].line, 5);
        assert.strictEqual(result[0].column, 1);
        assert.strictEqual(result[0].severity, 'warning');
        assert.strictEqual(result[0].message, 'Unused import detected');
        assert.strictEqual(result[0].rule, 'UnusedImport');
    });

    test('parses detekt without brackets', () => {
        const output = 'src/main.kt:5:1: UnusedImport - Unused import detected';
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].line, 5);
        assert.strictEqual(result[0].column, 1);
        assert.strictEqual(result[0].severity, 'warning');
        assert.strictEqual(result[0].message, 'Unused import detected');
        assert.strictEqual(result[0].rule, 'UnusedImport');
    });

    test('returns empty array for empty input', () => {
        assert.deepStrictEqual(parseDetektDiagnostics(''), []);
    });

    test('all detekt diagnostics have warning severity', () => {
        const output = [
            'src/a.kt:1:1: Magic number [MagicNumber]',
            'src/b.kt:2:1: LongMethod - Method too long [detekt.complexity]',
            'src/c.kt:3:1: EmptyBlock - Empty block [detekt.empty]',
        ].join('\n');
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 3);
        for (const diag of result) {
            assert.strictEqual(diag.severity, 'warning');
        }
    });

    test('parses paths with spaces', () => {
        const output = 'my dir/src/main.kt:3:5: Magic number [MagicNumber]';
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'my dir/src/main.kt');
        assert.strictEqual(result[0].line, 3);
        assert.strictEqual(result[0].column, 5);
        assert.strictEqual(result[0].message, 'Magic number');
        assert.strictEqual(result[0].rule, 'MagicNumber');
    });

    test('parses rule names with underscores and digits', () => {
        const output = 'src/main.kt:3:5: Msg [Rule_123]';
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].message, 'Msg');
        assert.strictEqual(result[0].rule, 'Rule_123');
    });

    test('captures last bracket group as rule when message contains brackets', () => {
        const output = 'src/main.kt:3:5: found [unused] import [UnusedImport]';
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].message, 'found [unused] import');
        assert.strictEqual(result[0].rule, 'UnusedImport');
    });

    test('handles Windows \\r\\n line endings', () => {
        const output = 'src/main.kt:3:5: Magic number [MagicNumber]\r\n';
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 1);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].message, 'Magic number');
        assert.strictEqual(result[0].rule, 'MagicNumber');
    });

    test('filters out non-diagnostic summary lines in mixed output', () => {
        const output = [
            'detekt finished in 1234ms',
            'src/main.kt:3:5: Magic number [MagicNumber]',
            '',
            'Overall debt: 10min',
            'src/util.kt:7:1: LongMethod - Method too long [detekt.complexity]',
            'Complexity report:',
        ].join('\n');
        const result = parseDetektDiagnostics(output);
        assert.strictEqual(result.length, 2);
        assert.strictEqual(result[0].file, 'src/main.kt');
        assert.strictEqual(result[0].rule, 'MagicNumber');
        assert.strictEqual(result[1].file, 'src/util.kt');
        assert.strictEqual(result[1].rule, 'LongMethod');
    });
});
