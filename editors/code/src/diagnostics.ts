import * as vscode from 'vscode';
import * as path from 'path';

let diagnosticCollection: vscode.DiagnosticCollection | undefined;

export function getDiagnosticCollection(): vscode.DiagnosticCollection {
    if (!diagnosticCollection) {
        diagnosticCollection = vscode.languages.createDiagnosticCollection('konvoy');
    }
    return diagnosticCollection;
}

export function disposeDiagnosticCollection(): void {
    diagnosticCollection?.dispose();
    diagnosticCollection = undefined;
}

export interface ParsedDiagnostic {
    file?: string;
    line?: number;    // 1-based from konanc
    column?: number;  // 1-based from konanc
    severity: 'error' | 'warning' | 'info';
    message: string;
    rule?: string;    // detekt rule name
}

const LOCATED_RE = /^(.+\.kt):(\d+):(?:(\d+):)?\s*(error|warning|info):\s*(.*)$/;
const BARE_RE = /^(error|warning|info):\s*(.*)$/;

export function parseKonancDiagnostics(output: string): ParsedDiagnostic[] {
    const diagnostics: ParsedDiagnostic[] = [];
    for (const line of output.split('\n')) {
        const trimmed = line.trim();
        if (!trimmed) {
            continue;
        }

        const located = LOCATED_RE.exec(trimmed);
        if (located) {
            diagnostics.push({
                file: located[1],
                line: parseInt(located[2], 10),
                column: located[3] ? parseInt(located[3], 10) : undefined,
                severity: located[4] as 'error' | 'warning' | 'info',
                message: located[5],
            });
            continue;
        }

        const bare = BARE_RE.exec(trimmed);
        if (bare) {
            diagnostics.push({
                severity: bare[1] as 'error' | 'warning' | 'info',
                message: bare[2],
            });
        }
    }
    return diagnostics;
}

// Real detekt 1.23.x: file.kt:3:5: message text [RuleName]
const DETEKT_REAL_RE = /^(.+\.kt):(\d+):(\d+):\s*(.+?)\s*\[(\w+)\]$/;
// Legacy: file.kt:3:5: RuleName - message [detekt.RuleSet]
const DETEKT_LEGACY_RE = /^(.+\.kt):(\d+):(\d+):\s*(\w+)\s*-\s*(.+?)\s*\[detekt\.\w+\]$/;
// Detekt without brackets: file.kt:3:5: RuleName - message
const DETEKT_BARE_RE = /^(.+\.kt):(\d+):(\d+):\s*(\w+)\s*-\s*(.+)$/;

export function parseDetektDiagnostics(output: string): ParsedDiagnostic[] {
    const diagnostics: ParsedDiagnostic[] = [];
    for (const line of output.split('\n')) {
        const trimmed = line.trim();
        if (!trimmed) {
            continue;
        }

        const legacy = DETEKT_LEGACY_RE.exec(trimmed);
        if (legacy) {
            diagnostics.push({
                file: legacy[1],
                line: parseInt(legacy[2], 10),
                column: parseInt(legacy[3], 10),
                severity: 'warning',
                message: legacy[5],
                rule: legacy[4],
            });
            continue;
        }

        const real = DETEKT_REAL_RE.exec(trimmed);
        if (real) {
            diagnostics.push({
                file: real[1],
                line: parseInt(real[2], 10),
                column: parseInt(real[3], 10),
                severity: 'warning',
                message: real[4],
                rule: real[5],
            });
            continue;
        }

        const bare = DETEKT_BARE_RE.exec(trimmed);
        if (bare) {
            diagnostics.push({
                file: bare[1],
                line: parseInt(bare[2], 10),
                column: parseInt(bare[3], 10),
                severity: 'warning',
                message: bare[5],
                rule: bare[4],
            });
        }
    }
    return diagnostics;
}

export function applyDiagnostics(
    workspaceRoot: string,
    diagnostics: ParsedDiagnostic[],
    collection: vscode.DiagnosticCollection,
): void {
    collection.clear();

    const grouped = new Map<string, vscode.Diagnostic[]>();

    for (const diag of diagnostics) {
        if (!diag.file) {
            continue;
        }

        const filePath = path.isAbsolute(diag.file)
            ? diag.file
            : path.resolve(workspaceRoot, diag.file);

        const line = Math.max((diag.line ?? 1) - 1, 0);
        const col = Math.max((diag.column ?? 1) - 1, 0);
        const range = new vscode.Range(line, col, line, col);

        let severity: vscode.DiagnosticSeverity;
        switch (diag.severity) {
            case 'error':
                severity = vscode.DiagnosticSeverity.Error;
                break;
            case 'warning':
                severity = vscode.DiagnosticSeverity.Warning;
                break;
            default:
                severity = vscode.DiagnosticSeverity.Information;
                break;
        }

        const message = diag.rule ? `[${diag.rule}] ${diag.message}` : diag.message;
        const vsDiag = new vscode.Diagnostic(range, message, severity);
        vsDiag.source = 'konvoy';

        const existing = grouped.get(filePath) ?? [];
        existing.push(vsDiag);
        grouped.set(filePath, existing);
    }

    for (const [filePath, diags] of grouped) {
        collection.set(vscode.Uri.file(filePath), diags);
    }
}

export function clearDiagnostics(collection: vscode.DiagnosticCollection): void {
    collection.clear();
}
