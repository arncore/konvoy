import * as assert from 'assert';
import * as vscode from 'vscode';
import * as path from 'path';
import { parseKonancDiagnostics, parseDetektDiagnostics } from '../../diagnostics';

// eslint-disable-next-line @typescript-eslint/no-var-requires
const { applyDiagnostics, clearDiagnostics, getDiagnosticCollection } = require('../../diagnostics');

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

suite('applyDiagnostics', () => {
    let collection: vscode.DiagnosticCollection;

    suiteSetup(() => {
        collection = vscode.languages.createDiagnosticCollection('test-konvoy-apply');
    });

    suiteTeardown(() => {
        collection.dispose();
    });

    setup(() => {
        collection.clear();
    });

    test('groups diagnostics by file correctly', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'error' as const, message: 'err1' },
            { file: 'src/util.kt', line: 2, column: 1, severity: 'warning' as const, message: 'warn1' },
            { file: 'src/main.kt', line: 5, column: 3, severity: 'error' as const, message: 'err2' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const mainUri = vscode.Uri.file('/workspace/src/main.kt');
        const utilUri = vscode.Uri.file('/workspace/src/util.kt');
        const mainDiags = collection.get(mainUri);
        const utilDiags = collection.get(utilUri);

        assert.strictEqual(mainDiags?.length, 2, 'main.kt should have 2 diagnostics');
        assert.strictEqual(utilDiags?.length, 1, 'util.kt should have 1 diagnostic');
    });

    test('converts 1-based line/column to 0-based VS Code ranges', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 10, column: 5, severity: 'error' as const, message: 'test' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags.length, 1);
        assert.strictEqual(diags[0].range.start.line, 9, 'line should be 0-based (10 -> 9)');
        assert.strictEqual(diags[0].range.start.character, 4, 'column should be 0-based (5 -> 4)');
    });

    test('defaults line and column to 0 when not provided', () => {
        const diagnostics = [
            { file: 'src/main.kt', severity: 'error' as const, message: 'no position' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags.length, 1);
        assert.strictEqual(diags[0].range.start.line, 0, 'line should default to 0');
        assert.strictEqual(diags[0].range.start.character, 0, 'column should default to 0');
    });

    test('maps error severity to DiagnosticSeverity.Error', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'error' as const, message: 'err' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags[0].severity, vscode.DiagnosticSeverity.Error);
    });

    test('maps warning severity to DiagnosticSeverity.Warning', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'warning' as const, message: 'warn' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags[0].severity, vscode.DiagnosticSeverity.Warning);
    });

    test('maps info severity to DiagnosticSeverity.Information', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'info' as const, message: 'info' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags[0].severity, vscode.DiagnosticSeverity.Information);
    });

    test('handles relative file paths by joining with workspaceRoot', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'error' as const, message: 'test' },
        ];
        applyDiagnostics('/my/project', diagnostics, collection);

        const expectedPath = path.resolve('/my/project', 'src/main.kt');
        const uri = vscode.Uri.file(expectedPath);
        const diags = collection.get(uri);
        assert.ok(diags, 'diagnostics should be set for resolved relative path');
        assert.strictEqual(diags!.length, 1);
    });

    test('handles absolute file paths without joining', () => {
        const absoluteFile = '/absolute/path/src/main.kt';
        const diagnostics = [
            { file: absoluteFile, line: 1, column: 1, severity: 'error' as const, message: 'test' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file(absoluteFile);
        const diags = collection.get(uri);
        assert.ok(diags, 'diagnostics should be set for the absolute path as-is');
        assert.strictEqual(diags!.length, 1);
    });

    test('skips diagnostics without a file', () => {
        const diagnostics = [
            { severity: 'error' as const, message: 'no file' },
            { file: 'src/main.kt', line: 1, column: 1, severity: 'error' as const, message: 'has file' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        // Only the diagnostic with a file should appear
        let count = 0;
        collection.forEach(() => { count++; });
        assert.strictEqual(count, 1, 'only diagnostics with a file should be applied');
    });

    test('sets source to konvoy on each diagnostic', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'error' as const, message: 'err' },
            { file: 'src/util.kt', line: 2, column: 1, severity: 'warning' as const, message: 'warn' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        collection.forEach((_uri, diags) => {
            for (const d of diags) {
                assert.strictEqual(d.source, 'konvoy', 'source should be konvoy');
            }
        });
    });

    test('multiple diagnostics for the same file are grouped together', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'error' as const, message: 'first' },
            { file: 'src/main.kt', line: 5, column: 1, severity: 'warning' as const, message: 'second' },
            { file: 'src/main.kt', line: 10, column: 1, severity: 'info' as const, message: 'third' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags.length, 3, 'all three diagnostics should be grouped under the same file');
        assert.strictEqual(diags[0].message, 'first');
        assert.strictEqual(diags[1].message, 'second');
        assert.strictEqual(diags[2].message, 'third');
    });

    test('clears existing diagnostics before applying new ones', () => {
        // Apply first batch
        const first = [
            { file: 'src/old.kt', line: 1, column: 1, severity: 'error' as const, message: 'old' },
        ];
        applyDiagnostics('/workspace', first, collection);

        const oldUri = vscode.Uri.file('/workspace/src/old.kt');
        assert.strictEqual(collection.get(oldUri)?.length, 1);

        // Apply second batch (should clear old ones)
        const second = [
            { file: 'src/new.kt', line: 1, column: 1, severity: 'error' as const, message: 'new' },
        ];
        applyDiagnostics('/workspace', second, collection);

        const oldDiags = collection.get(oldUri);
        assert.ok(!oldDiags || oldDiags.length === 0, 'old diagnostics should be cleared');

        const newUri = vscode.Uri.file('/workspace/src/new.kt');
        assert.strictEqual(collection.get(newUri)?.length, 1);
    });

    test('prepends rule name in brackets when rule is present', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'warning' as const, message: 'Magic number', rule: 'MagicNumber' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags[0].message, '[MagicNumber] Magic number');
    });

    test('does not prepend brackets when rule is absent', () => {
        const diagnostics = [
            { file: 'src/main.kt', line: 1, column: 1, severity: 'error' as const, message: 'plain error' },
        ];
        applyDiagnostics('/workspace', diagnostics, collection);

        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diags = collection.get(uri)!;
        assert.strictEqual(diags[0].message, 'plain error');
    });

    test('applies empty diagnostics array without error', () => {
        applyDiagnostics('/workspace', [], collection);

        let count = 0;
        collection.forEach(() => { count++; });
        assert.strictEqual(count, 0, 'collection should be empty when no diagnostics are provided');
    });
});

suite('clearDiagnostics', () => {
    let collection: vscode.DiagnosticCollection;

    suiteSetup(() => {
        collection = vscode.languages.createDiagnosticCollection('test-konvoy-clear');
    });

    suiteTeardown(() => {
        collection.dispose();
    });

    test('calling clear empties the collection', () => {
        // Populate the collection first
        const uri = vscode.Uri.file('/workspace/src/main.kt');
        const diag = new vscode.Diagnostic(
            new vscode.Range(0, 0, 0, 0),
            'test error',
            vscode.DiagnosticSeverity.Error,
        );
        collection.set(uri, [diag]);

        // Verify it has content
        assert.strictEqual(collection.get(uri)?.length, 1, 'collection should have 1 diagnostic before clear');

        // Clear it
        clearDiagnostics(collection);

        // Verify it is empty
        const afterClear = collection.get(uri);
        assert.ok(!afterClear || afterClear.length === 0, 'collection should be empty after clear');
    });

    test('clearing an already empty collection does not throw', () => {
        collection.clear(); // ensure empty
        assert.doesNotThrow(() => {
            clearDiagnostics(collection);
        });
    });
});

suite('getDiagnosticCollection', () => {
    test('returns a DiagnosticCollection', () => {
        const collection = getDiagnosticCollection();
        assert.ok(collection, 'getDiagnosticCollection must return a truthy value');
        assert.ok(typeof collection.set === 'function', 'must have a set method');
        assert.ok(typeof collection.clear === 'function', 'must have a clear method');
        assert.ok(typeof collection.dispose === 'function', 'must have a dispose method');
        assert.ok(typeof collection.forEach === 'function', 'must have a forEach method');
    });

    test('returns the same instance on subsequent calls (singleton)', () => {
        const first = getDiagnosticCollection();
        const second = getDiagnosticCollection();
        assert.strictEqual(first, second, 'getDiagnosticCollection should return the same instance');
    });
});
