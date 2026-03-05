import * as vscode from 'vscode';

// ── Types ──────────────────────────────────────────────────────────────────

export interface TomlDiagnostic {
    line: number;    // 0-based
    col: number;     // 0-based
    endCol: number;  // 0-based, exclusive
    message: string;
    severity: vscode.DiagnosticSeverity;
}

interface ParsedSection {
    name: string;
    line: number;
}

interface ParsedKeyValue {
    key: string;
    value: string;
    line: number;
    keyStart: number;
    keyEnd: number;
    valueStart: number;
    valueEnd: number;
}

// ── Regex patterns ─────────────────────────────────────────────────────────

const SECTION_RE = /^\s*\[([^\]]+)\]\s*$/;
const KEY_VALUE_RE = /^\s*(\w[\w-]*)\s*=\s*(.+)$/;
const VALID_NAME_RE = /^[a-zA-Z_][a-zA-Z0-9_-]*$/;

// ── Pure validation logic (testable without VS Code) ───────────────────────

export function validateManifest(text: string): TomlDiagnostic[] {
    const lines = text.split('\n');
    const diagnostics: TomlDiagnostic[] = [];

    const sections: ParsedSection[] = [];
    const kvBySection = new Map<string, ParsedKeyValue[]>();

    let currentSection = '';

    for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const trimmed = line.trim();

        // Skip empty lines and comments
        if (!trimmed || trimmed.startsWith('#')) {
            continue;
        }

        const sectionMatch = SECTION_RE.exec(trimmed);
        if (sectionMatch) {
            currentSection = sectionMatch[1].trim();
            sections.push({ name: currentSection, line: i });
            if (!kvBySection.has(currentSection)) {
                kvBySection.set(currentSection, []);
            }
            continue;
        }

        const kvMatch = KEY_VALUE_RE.exec(line);
        if (kvMatch) {
            const key = kvMatch[1];
            const value = kvMatch[2].trim();
            const keyStart = line.indexOf(key);
            const keyEnd = keyStart + key.length;
            const eqIndex = line.indexOf('=', keyEnd);
            const valueStart = eqIndex + 1 + (line.substring(eqIndex + 1).length - line.substring(eqIndex + 1).trimStart().length);
            const valueEnd = valueStart + value.length;

            const kv: ParsedKeyValue = { key, value, line: i, keyStart, keyEnd, valueStart, valueEnd };

            if (!kvBySection.has(currentSection)) {
                kvBySection.set(currentSection, []);
            }
            kvBySection.get(currentSection)!.push(kv);
        }
    }

    // Check required sections
    const sectionNames = sections.map(s => s.name);

    if (!sectionNames.includes('package')) {
        diagnostics.push({
            line: 0, col: 0, endCol: 1,
            message: 'Missing required section: [package]',
            severity: vscode.DiagnosticSeverity.Error,
        });
    }

    if (!sectionNames.includes('toolchain')) {
        diagnostics.push({
            line: 0, col: 0, endCol: 1,
            message: 'Missing required section: [toolchain]',
            severity: vscode.DiagnosticSeverity.Error,
        });
    }

    // Validate [package]
    const packageKvs = kvBySection.get('package') ?? [];
    const packageKeys = new Map(packageKvs.map(kv => [kv.key, kv]));

    const nameKv = packageKeys.get('name');
    if (nameKv) {
        const nameVal = stripQuotes(nameKv.value);
        if (nameVal.length === 0) {
            diagnostics.push({
                line: nameKv.line, col: nameKv.valueStart, endCol: nameKv.valueEnd,
                message: 'Package name must not be empty.',
                severity: vscode.DiagnosticSeverity.Error,
            });
        } else if (!VALID_NAME_RE.test(nameVal)) {
            diagnostics.push({
                line: nameKv.line, col: nameKv.valueStart, endCol: nameKv.valueEnd,
                message: 'Package name must match ^[a-zA-Z_][a-zA-Z0-9_-]*$',
                severity: vscode.DiagnosticSeverity.Error,
            });
        }
    } else if (sectionNames.includes('package')) {
        const pkgSection = sections.find(s => s.name === 'package')!;
        diagnostics.push({
            line: pkgSection.line, col: 0, endCol: lines[pkgSection.line].length,
            message: 'Missing required key: name',
            severity: vscode.DiagnosticSeverity.Error,
        });
    }

    const kindKv = packageKeys.get('kind');
    if (kindKv) {
        const kindVal = stripQuotes(kindKv.value);
        if (kindVal !== 'bin' && kindVal !== 'lib') {
            diagnostics.push({
                line: kindKv.line, col: kindKv.valueStart, endCol: kindKv.valueEnd,
                message: "Package kind must be \"bin\" or \"lib\".",
                severity: vscode.DiagnosticSeverity.Error,
            });
        }
    }

    const entryKv = packageKeys.get('entrypoint');
    if (entryKv) {
        const entryVal = stripQuotes(entryKv.value);
        const effectiveKind = kindKv ? stripQuotes(kindKv.value) : 'bin';
        if (effectiveKind === 'bin' && !entryVal.endsWith('.kt')) {
            diagnostics.push({
                line: entryKv.line, col: entryKv.valueStart, endCol: entryKv.valueEnd,
                message: 'Entrypoint for bin projects must end with .kt',
                severity: vscode.DiagnosticSeverity.Error,
            });
        }
    }

    // Validate [toolchain]
    const toolchainKvs = kvBySection.get('toolchain') ?? [];
    const toolchainKeys = new Map(toolchainKvs.map(kv => [kv.key, kv]));

    if (sectionNames.includes('toolchain')) {
        const kotlinKv = toolchainKeys.get('kotlin');
        if (!kotlinKv) {
            const tcSection = sections.find(s => s.name === 'toolchain')!;
            diagnostics.push({
                line: tcSection.line, col: 0, endCol: lines[tcSection.line].length,
                message: 'Missing required key: kotlin',
                severity: vscode.DiagnosticSeverity.Error,
            });
        } else {
            const kotlinVal = stripQuotes(kotlinKv.value);
            if (kotlinVal.length === 0) {
                diagnostics.push({
                    line: kotlinKv.line, col: kotlinKv.valueStart, endCol: kotlinKv.valueEnd,
                    message: 'Kotlin version must not be empty.',
                    severity: vscode.DiagnosticSeverity.Error,
                });
            }
        }

        const detektKv = toolchainKeys.get('detekt');
        if (detektKv) {
            const detektVal = stripQuotes(detektKv.value);
            if (detektVal.length === 0) {
                diagnostics.push({
                    line: detektKv.line, col: detektKv.valueStart, endCol: detektKv.valueEnd,
                    message: 'Detekt version must not be empty.',
                    severity: vscode.DiagnosticSeverity.Error,
                });
            }
        }
    }

    // Validate inline-style plugins (key = { maven = "...", version = "..." })
    const pluginsKvs = kvBySection.get('plugins') ?? [];
    for (const kv of pluginsKvs) {
        const val = kv.value.trim();
        if (val.startsWith('{') && val.endsWith('}')) {
            const hasMaven = /\bmaven\s*=/.test(val);
            const hasVersion = /\bversion\s*=/.test(val);
            if (!hasMaven) {
                diagnostics.push({
                    line: kv.line, col: kv.valueStart, endCol: kv.valueEnd,
                    message: `Plugin "${kv.key}" must have "maven" set to a groupId:artifactId coordinate.`,
                    severity: vscode.DiagnosticSeverity.Error,
                });
            }
            if (!hasVersion) {
                diagnostics.push({
                    line: kv.line, col: kv.valueStart, endCol: kv.valueEnd,
                    message: `Plugin "${kv.key}" must have "version" set.`,
                    severity: vscode.DiagnosticSeverity.Error,
                });
            }
        }
    }

    // Validate [plugins.*] sub-table format
    for (const section of sections) {
        if (!section.name.startsWith('plugins.')) {
            continue;
        }
        const pluginName = section.name.substring('plugins.'.length);
        const pluginKvs = kvBySection.get(section.name) ?? [];
        const pluginKeys = new Map(pluginKvs.map(kv => [kv.key, kv]));
        const hasMaven = pluginKeys.has('maven');
        const hasVersion = pluginKeys.has('version');

        if (!hasMaven) {
            diagnostics.push({
                line: section.line, col: 0, endCol: lines[section.line].length,
                message: `Plugin "${pluginName}" must have "maven" set to a groupId:artifactId coordinate.`,
                severity: vscode.DiagnosticSeverity.Error,
            });
        }
        if (!hasVersion) {
            diagnostics.push({
                line: section.line, col: 0, endCol: lines[section.line].length,
                message: `Plugin "${pluginName}" must have "version" set.`,
                severity: vscode.DiagnosticSeverity.Error,
            });
        }
    }

    // Validate [dependencies.*]
    for (const section of sections) {
        if (!section.name.startsWith('dependencies.')) {
            continue;
        }
        const depName = section.name.substring('dependencies.'.length);
        if (!VALID_NAME_RE.test(depName)) {
            diagnostics.push({
                line: section.line, col: 0, endCol: lines[section.line].length,
                message: `Dependency name "${depName}" must match ^[a-zA-Z_][a-zA-Z0-9_-]*$`,
                severity: vscode.DiagnosticSeverity.Error,
            });
        }

        const depKvs = kvBySection.get(section.name) ?? [];
        const depKeys = new Map(depKvs.map(kv => [kv.key, kv]));
        const hasPath = depKeys.has('path');
        const hasVersion = depKeys.has('version');

        if (!hasPath && !hasVersion) {
            diagnostics.push({
                line: section.line, col: 0, endCol: lines[section.line].length,
                message: `Dependency "${depName}" must have either "path" or "version".`,
                severity: vscode.DiagnosticSeverity.Error,
            });
        } else if (hasPath && hasVersion) {
            diagnostics.push({
                line: section.line, col: 0, endCol: lines[section.line].length,
                message: `Dependency "${depName}" must have only one of "path" or "version", not both.`,
                severity: vscode.DiagnosticSeverity.Error,
            });
        }
    }

    // Also validate inline-style dependencies (key = { path = "..." } or key = { version = "..." })
    const depsKvs = kvBySection.get('dependencies') ?? [];
    for (const kv of depsKvs) {
        if (!VALID_NAME_RE.test(kv.key)) {
            diagnostics.push({
                line: kv.line, col: kv.keyStart, endCol: kv.keyEnd,
                message: `Dependency name "${kv.key}" must match ^[a-zA-Z_][a-zA-Z0-9_-]*$`,
                severity: vscode.DiagnosticSeverity.Error,
            });
        }

        // Simple check for inline table dependencies
        const val = kv.value.trim();
        if (val.startsWith('{') && val.endsWith('}')) {
            const hasPath = /\bpath\s*=/.test(val);
            const hasVersion = /\bversion\s*=/.test(val);
            if (!hasPath && !hasVersion) {
                diagnostics.push({
                    line: kv.line, col: kv.valueStart, endCol: kv.valueEnd,
                    message: `Dependency "${kv.key}" must have either "path" or "version".`,
                    severity: vscode.DiagnosticSeverity.Error,
                });
            } else if (hasPath && hasVersion) {
                diagnostics.push({
                    line: kv.line, col: kv.valueStart, endCol: kv.valueEnd,
                    message: `Dependency "${kv.key}" must have only one of "path" or "version", not both.`,
                    severity: vscode.DiagnosticSeverity.Error,
                });
            }
        }
    }

    return diagnostics;
}

function stripQuotes(s: string): string {
    if ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'"))) {
        return s.slice(1, -1);
    }
    return s;
}

// ── Section detection for completions ──────────────────────────────────────

function getCurrentSection(document: vscode.TextDocument, position: vscode.Position): string {
    for (let i = position.line; i >= 0; i--) {
        const match = SECTION_RE.exec(document.lineAt(i).text.trim());
        if (match) {
            return match[1].trim();
        }
    }
    return '';
}

// ── Hover documentation ────────────────────────────────────────────────────

const KEY_DOCS: Record<string, Record<string, string>> = {
    'package': {
        'name': 'Project name. Must match `^[a-zA-Z_][a-zA-Z0-9_-]*$`',
        'kind': "Package type: `'bin'` for executable, `'lib'` for library (.klib)",
        'version': 'Package version (optional)',
        'entrypoint': 'Entry point file. Default: `src/main.kt` (bin), `src/lib.kt` (lib)',
    },
    'toolchain': {
        'kotlin': 'Kotlin/Native version (e.g. `2.1.0`)',
        'detekt': 'Detekt version for linting (e.g. `1.23.7`). Enables `konvoy lint`',
    },
};

// ── Providers ──────────────────────────────────────────────────────────────

class TomlCompletionProvider implements vscode.CompletionItemProvider {
    provideCompletionItems(
        document: vscode.TextDocument,
        position: vscode.Position,
    ): vscode.CompletionItem[] {
        const section = getCurrentSection(document, position);
        const lineText = document.lineAt(position.line).text;

        if (!section) {
            // Top level: suggest section headers
            return [
                this.sectionItem('[package]', 'Package metadata'),
                this.sectionItem('[toolchain]', 'Toolchain versions'),
                this.sectionItem('[dependencies]', 'Project dependencies'),
                this.sectionItem('[plugins]', 'Compiler plugins'),
            ];
        }

        if (section === 'package') {
            // Check if completing a value for `kind =`
            if (/^\s*kind\s*=\s*/.test(lineText)) {
                return [
                    this.valueItem('"bin"', 'Native executable'),
                    this.valueItem('"lib"', 'Kotlin/Native library (.klib)'),
                ];
            }
            return [
                this.keyItem('name', 'Project name (required)'),
                this.keyItem('kind', 'Package type: bin or lib'),
                this.keyItem('version', 'Package version'),
                this.keyItem('entrypoint', 'Entry point file'),
            ];
        }

        if (section === 'toolchain') {
            return [
                this.keyItem('kotlin', 'Kotlin/Native version (required)'),
                this.keyItem('detekt', 'Detekt version for linting'),
            ];
        }

        return [];
    }

    private sectionItem(label: string, detail: string): vscode.CompletionItem {
        const item = new vscode.CompletionItem(label, vscode.CompletionItemKind.Module);
        item.detail = detail;
        item.insertText = new vscode.SnippetString(label + '\n$0');
        return item;
    }

    private keyItem(key: string, detail: string): vscode.CompletionItem {
        const item = new vscode.CompletionItem(key, vscode.CompletionItemKind.Property);
        item.detail = detail;
        item.insertText = new vscode.SnippetString(`${key} = "$1"$0`);
        return item;
    }

    private valueItem(label: string, detail: string): vscode.CompletionItem {
        const item = new vscode.CompletionItem(label, vscode.CompletionItemKind.Value);
        item.detail = detail;
        return item;
    }
}

class TomlHoverProvider implements vscode.HoverProvider {
    provideHover(
        document: vscode.TextDocument,
        position: vscode.Position,
    ): vscode.Hover | undefined {
        const section = getCurrentSection(document, position);
        const line = document.lineAt(position.line).text;

        const kvMatch = KEY_VALUE_RE.exec(line);
        if (!kvMatch) {
            return undefined;
        }

        const key = kvMatch[1];
        const keyStart = line.indexOf(key);
        const keyEnd = keyStart + key.length;

        // Only show hover when cursor is on the key
        if (position.character < keyStart || position.character > keyEnd) {
            return undefined;
        }

        const sectionDocs = KEY_DOCS[section];
        if (!sectionDocs) {
            return undefined;
        }

        const doc = sectionDocs[key];
        if (!doc) {
            return undefined;
        }

        return new vscode.Hover(
            new vscode.MarkdownString(doc),
            new vscode.Range(position.line, keyStart, position.line, keyEnd),
        );
    }
}

// ── Registration ───────────────────────────────────────────────────────────

const LANGUAGE_ID = 'konvoy-toml';

export function registerTomlSupport(context: vscode.ExtensionContext): vscode.Disposable[] {
    const diagnosticCollection = vscode.languages.createDiagnosticCollection('konvoy-toml');
    const disposables: vscode.Disposable[] = [diagnosticCollection];

    const selector: vscode.DocumentSelector = { language: LANGUAGE_ID };

    // Validate on save
    const onSave = vscode.workspace.onDidSaveTextDocument(doc => {
        if (doc.languageId === LANGUAGE_ID) {
            runValidation(doc, diagnosticCollection);
        }
    });
    disposables.push(onSave);

    // Validate on change (debounced via VS Code's event batching)
    const onChange = vscode.workspace.onDidChangeTextDocument(event => {
        if (event.document.languageId === LANGUAGE_ID) {
            runValidation(event.document, diagnosticCollection);
        }
    });
    disposables.push(onChange);

    // Clear diagnostics when the document is closed
    const onClose = vscode.workspace.onDidCloseTextDocument(doc => {
        if (doc.languageId === LANGUAGE_ID) {
            diagnosticCollection.delete(doc.uri);
        }
    });
    disposables.push(onClose);

    // Validate already-open konvoy.toml files
    for (const doc of vscode.workspace.textDocuments) {
        if (doc.languageId === LANGUAGE_ID) {
            runValidation(doc, diagnosticCollection);
        }
    }

    // Completions
    const completionProvider = vscode.languages.registerCompletionItemProvider(
        selector,
        new TomlCompletionProvider(),
        '[', '=', '.',
    );
    disposables.push(completionProvider);

    // Hover
    const hoverProvider = vscode.languages.registerHoverProvider(
        selector,
        new TomlHoverProvider(),
    );
    disposables.push(hoverProvider);

    return disposables;
}

function runValidation(
    document: vscode.TextDocument,
    collection: vscode.DiagnosticCollection,
): void {
    const tomlDiags = validateManifest(document.getText());
    const vsDiags = tomlDiags.map(d => {
        const range = new vscode.Range(d.line, d.col, d.line, d.endCol);
        const diag = new vscode.Diagnostic(range, d.message, d.severity);
        diag.source = 'konvoy-toml';
        return diag;
    });
    collection.set(document.uri, vsDiags);
}
