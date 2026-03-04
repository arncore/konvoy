import * as assert from 'assert';
import * as vscode from 'vscode';
import { validateManifest, TomlDiagnostic } from '../../tomlSupport';

function hasError(diags: TomlDiagnostic[], substring: string): boolean {
    return diags.some(
        d => d.severity === vscode.DiagnosticSeverity.Error && d.message.includes(substring),
    );
}

const VALID_MINIMAL = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"
`.trim();

const VALID_FULL = `
[package]
name = "my-app"
kind = "bin"
version = "1.0.0"
entrypoint = "src/main.kt"

[toolchain]
kotlin = "2.1.0"
detekt = "1.23.7"

[dependencies]
mylib = { path = "../mylib" }

[plugins.serialization]
version = "2.1.0"
`.trim();

suite('validateManifest', () => {
    test('valid minimal manifest passes validation', () => {
        const diags = validateManifest(VALID_MINIMAL);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });

    test('missing [package] section is flagged', () => {
        const text = `
[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Missing required section: [package]'));
    });

    test('missing [toolchain] section is flagged', () => {
        const text = `
[package]
name = "hello"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Missing required section: [toolchain]'));
    });

    test('empty package name is flagged', () => {
        const text = `
[package]
name = ""

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Package name must not be empty'));
    });

    test('invalid package name characters are flagged', () => {
        const text = `
[package]
name = "hello world!"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Package name must match'));
    });

    test('invalid kind value is flagged', () => {
        const text = `
[package]
name = "hello"
kind = "jar"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Package kind must be'));
    });

    test('missing .kt extension on entrypoint for bin project is flagged', () => {
        const text = `
[package]
name = "hello"
kind = "bin"
entrypoint = "src/main.kts"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Entrypoint for bin projects must end with .kt'));
    });

    test('empty kotlin version is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = ""
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Kotlin version must not be empty'));
    });

    test('empty detekt version is flagged when key is present', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"
detekt = ""
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Detekt version must not be empty'));
    });

    test('valid manifest with all sections passes', () => {
        const diags = validateManifest(VALID_FULL);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });

    test('lib kind entrypoint without .kt is allowed', () => {
        const text = `
[package]
name = "mylib"
kind = "lib"
entrypoint = "src/lib.klib"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        // Should not flag entrypoint for lib projects
        assert.ok(!hasError(diags, 'Entrypoint'));
    });

    test('dependency with both path and version is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies]
mylib = { path = "../mylib", version = "1.0" }
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'only one of'));
    });

    test('dependency with neither path nor version is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies]
mylib = { something = "else" }
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'must have either'));
    });
});
