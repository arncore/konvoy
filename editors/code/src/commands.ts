import * as vscode from 'vscode';
import * as cp from 'child_process';
import { getKonvoyPath } from './konvoyBinary';
import { getOutputChannel } from './outputChannel';
import { getBestWorkspaceFolder } from './workspaceDetector';
import {
    getDiagnosticCollection,
    parseKonancDiagnostics,
    parseDetektDiagnostics,
    applyDiagnostics,
    clearDiagnostics,
} from './diagnostics';

interface CommandConfig {
    id: string;
    args: string[];
    parseDiagnostics: boolean;
    useDetektParser: boolean;
}

export const COMMANDS: CommandConfig[] = [
    { id: 'konvoy.build',            args: ['build', '--verbose'],               parseDiagnostics: true,  useDetektParser: false },
    { id: 'konvoy.buildRelease',     args: ['build', '--release', '--verbose'],  parseDiagnostics: true,  useDetektParser: false },
    { id: 'konvoy.run',              args: ['run', '--verbose'],                 parseDiagnostics: true,  useDetektParser: false },
    { id: 'konvoy.runRelease',       args: ['run', '--release', '--verbose'],    parseDiagnostics: true,  useDetektParser: false },
    { id: 'konvoy.test',             args: ['test', '--verbose'],                parseDiagnostics: true,  useDetektParser: false },
    { id: 'konvoy.lint',             args: ['lint', '--verbose'],                parseDiagnostics: true,  useDetektParser: true  },
    { id: 'konvoy.update',           args: ['update'],                           parseDiagnostics: false, useDetektParser: false },
    { id: 'konvoy.clean',            args: ['clean'],                            parseDiagnostics: false, useDetektParser: false },
    { id: 'konvoy.cleanAll',         args: ['clean', '--all'],                   parseDiagnostics: false, useDetektParser: false },
    { id: 'konvoy.doctor',           args: ['doctor'],                           parseDiagnostics: false, useDetektParser: false },
    { id: 'konvoy.toolchainInstall', args: ['toolchain', 'install'],             parseDiagnostics: false, useDetektParser: false },
    { id: 'konvoy.toolchainList',    args: ['toolchain', 'list'],                parseDiagnostics: false, useDetektParser: false },
];

let runningProcess: cp.ChildProcess | undefined;

function runCommand(config: CommandConfig): void {
    if (runningProcess) {
        vscode.window.showWarningMessage('A konvoy command is already running. Wait for it to finish.');
        return;
    }

    const folder = getBestWorkspaceFolder();
    if (!folder) {
        vscode.window.showErrorMessage('No workspace folder found. Open a folder containing konvoy.toml.');
        return;
    }

    const konvoyPath = getKonvoyPath();
    const output = getOutputChannel();
    output.clear();
    output.appendLine(`> ${konvoyPath} ${config.args.join(' ')}`);
    output.show(true);

    if (config.parseDiagnostics) {
        clearDiagnostics(getDiagnosticCollection());
    }

    let proc: cp.ChildProcess;
    try {
        proc = cp.spawn(konvoyPath, config.args, {
            cwd: folder.uri.fsPath,
        });
    } catch {
        vscode.window.showErrorMessage(
            'konvoy binary not found. Set konvoy.path in settings or install konvoy.',
        );
        return;
    }

    runningProcess = proc;
    let stderrBuffer = '';

    proc.stdout?.on('data', (data: Buffer) => {
        output.append(data.toString());
    });

    proc.stderr?.on('data', (data: Buffer) => {
        const text = data.toString();
        output.append(text);
        if (config.parseDiagnostics) {
            stderrBuffer += text;
        }
    });

    proc.on('error', (err: Error) => {
        runningProcess = undefined;
        // ENOENT means the binary was not found on PATH
        if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
            vscode.window.showErrorMessage(
                'konvoy binary not found. Set konvoy.path in settings or install konvoy.',
            );
        } else {
            vscode.window.showErrorMessage(`Failed to start konvoy: ${err.message}`);
        }
    });

    proc.on('close', (code: number | null) => {
        runningProcess = undefined;

        if (config.parseDiagnostics) {
            const parser = config.useDetektParser ? parseDetektDiagnostics : parseKonancDiagnostics;
            const diagnostics = parser(stderrBuffer);
            applyDiagnostics(folder.uri.fsPath, diagnostics, getDiagnosticCollection());
        }

        if (code === null) {
            output.show(true);
            vscode.window.showWarningMessage(`konvoy ${config.args[0]} was terminated by a signal.`);
        } else if (code !== 0) {
            output.show(true);
            vscode.window.showErrorMessage(`konvoy ${config.args[0]} failed (exit code ${code}).`);
        } else {
            const showOnSuccess = vscode.workspace
                .getConfiguration('konvoy')
                .get<boolean>('showBuildOutputOnSuccess', false);
            if (!showOnSuccess) {
                // Don't force the output panel into view, but keep it available
            }
            vscode.window.showInformationMessage(`konvoy ${config.args[0]} succeeded.`);
        }
    });
}

export function registerCommands(): vscode.Disposable[] {
    const disposables = COMMANDS.map((config) =>
        vscode.commands.registerCommand(config.id, () => runCommand(config)),
    );

    const cleanConfig = COMMANDS.find((c) => c.id === 'konvoy.clean');
    const cleanAllConfig = COMMANDS.find((c) => c.id === 'konvoy.cleanAll');
    if (cleanConfig && cleanAllConfig) {
        disposables.push(
            vscode.commands.registerCommand('konvoy.cleanConfirm', async () => {
                const choice = await vscode.window.showWarningMessage(
                    'What do you want to clean?',
                    { modal: true },
                    'Clean (build only)',
                    'Clean All (.konvoy/)',
                );
                if (choice === 'Clean (build only)') {
                    runCommand(cleanConfig);
                } else if (choice === 'Clean All (.konvoy/)') {
                    runCommand(cleanAllConfig);
                }
            }),
        );
    }

    return disposables;
}

/** @internal Exposed for testing only. */
export const _testing = {
    isRunning: (): boolean => runningProcess !== undefined,
    setRunning: (proc: cp.ChildProcess | undefined): void => { runningProcess = proc; },
};
