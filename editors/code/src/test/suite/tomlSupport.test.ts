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

    test('completely empty input reports missing [package] and [toolchain]', () => {
        const diags = validateManifest('');
        assert.ok(hasError(diags, 'Missing required section: [package]'));
        assert.ok(hasError(diags, 'Missing required section: [toolchain]'));
    });

    test('comments-only input reports missing sections', () => {
        const diags = validateManifest('# just a comment');
        assert.ok(hasError(diags, 'Missing required section: [package]'));
        assert.ok(hasError(diags, 'Missing required section: [toolchain]'));
    });

    test('missing name key in [package] is flagged', () => {
        const text = `
[package]
kind = "bin"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Missing required key: name'));
    });

    test('missing kotlin key in [toolchain] is flagged', () => {
        const text = `
[package]
name = "app"

[toolchain]
detekt = "1.23.7"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Missing required key: kotlin'));
    });

    test('package name starting with digit fails regex', () => {
        const text = `
[package]
name = "123app"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Package name must match'));
    });

    test('package name starting with underscore is valid', () => {
        const text = `
[package]
name = "_app"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });

    test('single-quoted values are accepted', () => {
        const text = `
[package]
name = 'hello'

[toolchain]
kotlin = '2.1.0'
`.trim();
        const diags = validateManifest(text);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });

    test('plugin without version key is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[plugins.serialization]
modules = "json"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Missing required key: version'));
    });

    test('plugin with empty version is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[plugins.serialization]
version = ""
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Plugin version must not be empty'));
    });

    test('plugin with valid version passes', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[plugins.serialization]
version = "1.8.0"
`.trim();
        const diags = validateManifest(text);
        // Should not flag any plugin errors
        assert.ok(!hasError(diags, 'version'));
    });

    test('default bin entrypoint without .kt extension is flagged', () => {
        const text = `
[package]
name = "hello"
entrypoint = "lib.klib"

[toolchain]
kotlin = "2.1.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Entrypoint for bin projects must end with .kt'));
    });

    test('valid manifest with Maven dependency passes', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies]
mylib = { version = "1.0.0" }
`.trim();
        const diags = validateManifest(text);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });

    // ── Sub-table dependency format [dependencies.name] ──────────────────

    test('valid sub-table dependency with path passes', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies.mylib]
path = "../mylib"
`.trim();
        const diags = validateManifest(text);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });

    test('valid sub-table dependency with version passes', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies.mylib]
version = "1.0.0"
`.trim();
        const diags = validateManifest(text);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });

    test('sub-table dependency with neither path nor version is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies.mylib]
something = "else"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'must have either'));
    });

    test('sub-table dependency with both path and version is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies.mylib]
path = "../mylib"
version = "1.0.0"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'only one of'));
    });

    test('sub-table dependency with invalid name is flagged', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies.123bad]
path = "../mylib"
`.trim();
        const diags = validateManifest(text);
        assert.ok(hasError(diags, 'Dependency name "123bad" must match'));
    });

    test('multiple sub-table dependencies all validate independently', () => {
        const text = `
[package]
name = "hello"

[toolchain]
kotlin = "2.1.0"

[dependencies.libA]
path = "../libA"

[dependencies.libB]
version = "2.0.0"
`.trim();
        const diags = validateManifest(text);
        assert.strictEqual(diags.length, 0, `Expected no errors, got: ${JSON.stringify(diags)}`);
    });
});

// ── Helper to open a konvoy-toml document ─────────────────────────────────

async function openTomlDoc(content: string): Promise<vscode.TextDocument> {
    return vscode.workspace.openTextDocument({ content, language: 'konvoy-toml' });
}

// ── TomlProviders (shared registration) ───────────────────────────────────

suite('TomlProviders', () => {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const { registerTomlSupport } = require('../../tomlSupport');
    let tomlDisposables: vscode.Disposable[] = [];

    suiteSetup(() => {
        // registerTomlSupport expects an ExtensionContext but only uses it
        // for the return value; pass a minimal stub.
        tomlDisposables = registerTomlSupport({} as vscode.ExtensionContext);
    });

    suiteTeardown(() => {
        for (const d of tomlDisposables) {
            d.dispose();
        }
    });

    // ── TomlCompletionProvider ────────────────────────────────────────────

    suite('TomlCompletionProvider', () => {
        test('returns section completions at top level (empty bracket)', async () => {
            const doc = await openTomlDoc('[');
            const result = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                doc.uri,
                new vscode.Position(0, 1),
            );
            assert.ok(result, 'Expected a CompletionList');
            const labels = result.items.map(i => typeof i.label === 'string' ? i.label : i.label.label);
            assert.ok(labels.some(l => l.includes('[package]')), `Expected [package] in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.some(l => l.includes('[toolchain]')), `Expected [toolchain] in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.some(l => l.includes('[dependencies]')), `Expected [dependencies] in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.some(l => l.includes('[plugins]')), `Expected [plugins] in completions, got: ${JSON.stringify(labels)}`);
        });

        test('returns section completions for partial section header', async () => {
            const doc = await openTomlDoc('[p');
            const result = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                doc.uri,
                new vscode.Position(0, 2),
            );
            assert.ok(result, 'Expected a CompletionList');
            const labels = result.items.map(i => typeof i.label === 'string' ? i.label : i.label.label);
            // VS Code will filter by the partial text, but the provider should still offer section items
            assert.ok(labels.some(l => l.includes('[package]')), `Expected [package] in completions, got: ${JSON.stringify(labels)}`);
        });

        test('returns key completions inside [package] section', async () => {
            const content = '[package]\n';
            const doc = await openTomlDoc(content);
            const result = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                doc.uri,
                new vscode.Position(1, 0),
            );
            assert.ok(result, 'Expected a CompletionList');
            const labels = result.items.map(i => typeof i.label === 'string' ? i.label : i.label.label);
            assert.ok(labels.includes('name'), `Expected "name" in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.includes('kind'), `Expected "kind" in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.includes('version'), `Expected "version" in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.includes('entrypoint'), `Expected "entrypoint" in completions, got: ${JSON.stringify(labels)}`);
        });

        test('returns key completions inside [toolchain] section', async () => {
            const content = '[toolchain]\n';
            const doc = await openTomlDoc(content);
            const result = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                doc.uri,
                new vscode.Position(1, 0),
            );
            assert.ok(result, 'Expected a CompletionList');
            const labels = result.items.map(i => typeof i.label === 'string' ? i.label : i.label.label);
            assert.ok(labels.includes('kotlin'), `Expected "kotlin" in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.includes('detekt'), `Expected "detekt" in completions, got: ${JSON.stringify(labels)}`);
        });

        test('returns value completions for kind key', async () => {
            const content = '[package]\nkind = ';
            const doc = await openTomlDoc(content);
            const result = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                doc.uri,
                new vscode.Position(1, 7),
            );
            assert.ok(result, 'Expected a CompletionList');
            const labels = result.items.map(i => typeof i.label === 'string' ? i.label : i.label.label);
            assert.ok(labels.some(l => l.includes('"bin"')), `Expected "bin" value in completions, got: ${JSON.stringify(labels)}`);
            assert.ok(labels.some(l => l.includes('"lib"')), `Expected "lib" value in completions, got: ${JSON.stringify(labels)}`);
        });

        test('returns empty completions for unknown section', async () => {
            const content = '[unknown]\n';
            const doc = await openTomlDoc(content);
            const result = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                doc.uri,
                new vscode.Position(1, 0),
            );
            assert.ok(result, 'Expected a CompletionList');
            // The provider returns [] for unknown sections; VS Code may still include
            // word-based suggestions, so we check that none of our known keys appear
            // with CompletionItemKind.Property
            const propertyItems = result.items.filter(i => i.kind === vscode.CompletionItemKind.Property);
            assert.strictEqual(propertyItems.length, 0, `Expected no Property completions for unknown section, got: ${JSON.stringify(propertyItems.map(i => i.label))}`);
        });

        test('returns empty completions for [dependencies] section', async () => {
            const content = '[dependencies]\n';
            const doc = await openTomlDoc(content);
            const result = await vscode.commands.executeCommand<vscode.CompletionList>(
                'vscode.executeCompletionItemProvider',
                doc.uri,
                new vscode.Position(1, 0),
            );
            assert.ok(result, 'Expected a CompletionList');
            const propertyItems = result.items.filter(i => i.kind === vscode.CompletionItemKind.Property);
            assert.strictEqual(propertyItems.length, 0, `Expected no Property completions for [dependencies], got: ${JSON.stringify(propertyItems.map(i => i.label))}`);
        });
    });

    // ── TomlHoverProvider ─────────────────────────────────────────────────

    suite('TomlHoverProvider', () => {
        test('returns hover for "name" key in [package]', async () => {
            const content = '[package]\nname = "hello"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 1), // cursor on "name"
            );
            assert.ok(hovers && hovers.length > 0, 'Expected at least one hover');
            const hoverText = hovers.map(h => h.contents.map(c =>
                typeof c === 'string' ? c : (c as vscode.MarkdownString).value,
            ).join('')).join('');
            assert.ok(hoverText.includes('Project name'), `Expected hover to mention "Project name", got: ${hoverText}`);
        });

        test('returns hover for "version" key in [package]', async () => {
            const content = '[package]\nversion = "1.0.0"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 2), // cursor on "version"
            );
            assert.ok(hovers && hovers.length > 0, 'Expected at least one hover');
            const hoverText = hovers.map(h => h.contents.map(c =>
                typeof c === 'string' ? c : (c as vscode.MarkdownString).value,
            ).join('')).join('');
            assert.ok(hoverText.includes('Package version'), `Expected hover to mention "Package version", got: ${hoverText}`);
        });

        test('returns hover for "kotlin" key in [toolchain]', async () => {
            const content = '[toolchain]\nkotlin = "2.1.0"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 2), // cursor on "kotlin"
            );
            assert.ok(hovers && hovers.length > 0, 'Expected at least one hover');
            const hoverText = hovers.map(h => h.contents.map(c =>
                typeof c === 'string' ? c : (c as vscode.MarkdownString).value,
            ).join('')).join('');
            assert.ok(hoverText.includes('Kotlin/Native version'), `Expected hover to mention "Kotlin/Native version", got: ${hoverText}`);
        });

        test('returns hover for "kind" key in [package]', async () => {
            const content = '[package]\nkind = "bin"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 1), // cursor on "kind"
            );
            assert.ok(hovers && hovers.length > 0, 'Expected at least one hover');
            const hoverText = hovers.map(h => h.contents.map(c =>
                typeof c === 'string' ? c : (c as vscode.MarkdownString).value,
            ).join('')).join('');
            assert.ok(hoverText.includes('bin'), `Expected hover to mention "bin", got: ${hoverText}`);
            assert.ok(hoverText.includes('lib'), `Expected hover to mention "lib", got: ${hoverText}`);
        });

        test('returns hover for "detekt" key in [toolchain]', async () => {
            const content = '[toolchain]\ndetekt = "1.23.7"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 2), // cursor on "detekt"
            );
            assert.ok(hovers && hovers.length > 0, 'Expected at least one hover');
            const hoverText = hovers.map(h => h.contents.map(c =>
                typeof c === 'string' ? c : (c as vscode.MarkdownString).value,
            ).join('')).join('');
            assert.ok(hoverText.includes('Detekt'), `Expected hover to mention "Detekt", got: ${hoverText}`);
        });

        test('returns hover for "entrypoint" key in [package]', async () => {
            const content = '[package]\nentrypoint = "src/main.kt"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 3), // cursor on "entrypoint"
            );
            assert.ok(hovers && hovers.length > 0, 'Expected at least one hover');
            const hoverText = hovers.map(h => h.contents.map(c =>
                typeof c === 'string' ? c : (c as vscode.MarkdownString).value,
            ).join('')).join('');
            assert.ok(hoverText.includes('Entry point'), `Expected hover to mention "Entry point", got: ${hoverText}`);
        });

        test('returns no hover for unknown key', async () => {
            const content = '[package]\nunknown_key = "value"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 3), // cursor on "unknown_key"
            );
            // Provider returns undefined for unknown keys; executeHoverProvider returns empty array
            assert.ok(!hovers || hovers.length === 0, `Expected no hovers for unknown key, got: ${JSON.stringify(hovers)}`);
        });

        test('returns no hover for empty line', async () => {
            const content = '[package]\n\nname = "hello"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 0), // cursor on empty line
            );
            assert.ok(!hovers || hovers.length === 0, `Expected no hovers for empty line, got: ${JSON.stringify(hovers)}`);
        });

        test('returns no hover for comment line', async () => {
            const content = '[package]\n# this is a comment\nname = "hello"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 5), // cursor on comment
            );
            assert.ok(!hovers || hovers.length === 0, `Expected no hovers for comment, got: ${JSON.stringify(hovers)}`);
        });

        test('returns no hover when cursor is on the value, not the key', async () => {
            const content = '[package]\nname = "hello"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 10), // cursor on "hello" value
            );
            assert.ok(!hovers || hovers.length === 0, `Expected no hovers on value part, got: ${JSON.stringify(hovers)}`);
        });

        test('returns no hover for key in unknown section', async () => {
            const content = '[unknown]\nname = "hello"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(1, 1), // cursor on "name" under [unknown]
            );
            assert.ok(!hovers || hovers.length === 0, `Expected no hovers for key in unknown section, got: ${JSON.stringify(hovers)}`);
        });

        test('returns no hover for section header line itself', async () => {
            const content = '[package]\nname = "hello"';
            const doc = await openTomlDoc(content);
            const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
                'vscode.executeHoverProvider',
                doc.uri,
                new vscode.Position(0, 3), // cursor on "[package]"
            );
            // The section header line does not match KEY_VALUE_RE, so no hover
            assert.ok(!hovers || hovers.length === 0, `Expected no hovers for section header, got: ${JSON.stringify(hovers)}`);
        });
    });
});
