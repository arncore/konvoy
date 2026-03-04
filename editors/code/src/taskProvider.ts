import * as vscode from 'vscode';
import { getKonvoyPath } from './konvoyBinary';

interface KonvoyTaskDefinition extends vscode.TaskDefinition {
    command: string;
}

const TASK_CONFIGS = [
    { command: 'build',           label: 'build',           args: ['build', '--verbose'],              group: vscode.TaskGroup.Build, isDefault: true,  problemMatcher: '$konvoy-konanc' },
    { command: 'build-release',   label: 'build (release)', args: ['build', '--release', '--verbose'], group: vscode.TaskGroup.Build, isDefault: false, problemMatcher: '$konvoy-konanc' },
    { command: 'run',             label: 'run',             args: ['run', '--verbose'],                group: undefined,              isDefault: false, problemMatcher: '$konvoy-konanc' },
    { command: 'run-release',     label: 'run (release)',   args: ['run', '--release', '--verbose'],   group: undefined,              isDefault: false, problemMatcher: '$konvoy-konanc' },
    { command: 'test',            label: 'test',            args: ['test', '--verbose'],               group: vscode.TaskGroup.Test,  isDefault: false, problemMatcher: '$konvoy-konanc' },
    { command: 'lint',            label: 'lint',            args: ['lint', '--verbose'],               group: undefined,              isDefault: false, problemMatcher: '$konvoy-konanc' },
    { command: 'clean',           label: 'clean',           args: ['clean'],                           group: undefined,              isDefault: false, problemMatcher: '$konvoy-bare'   },
    { command: 'doctor',          label: 'doctor',          args: ['doctor'],                          group: undefined,              isDefault: false, problemMatcher: '$konvoy-bare'   },
];

class KonvoyTaskProvider implements vscode.TaskProvider {
    private tasks: vscode.Task[] | undefined;

    provideTasks(): vscode.Task[] {
        if (!this.tasks) {
            this.tasks = this.buildTasks();
        }
        return this.tasks;
    }

    resolveTask(task: vscode.Task): vscode.Task | undefined {
        const definition = task.definition as KonvoyTaskDefinition;
        const config = TASK_CONFIGS.find(c => c.command === definition.command);
        if (!config) {
            return undefined;
        }
        return this.createTask(config);
    }

    invalidate(): void {
        this.tasks = undefined;
    }

    private buildTasks(): vscode.Task[] {
        return TASK_CONFIGS.map(config => this.createTask(config));
    }

    private createTask(config: typeof TASK_CONFIGS[number]): vscode.Task {
        const definition: KonvoyTaskDefinition = {
            type: 'konvoy',
            command: config.command,
        };

        const execution = new vscode.ShellExecution(getKonvoyPath(), config.args);

        const task = new vscode.Task(
            definition,
            vscode.TaskScope.Workspace,
            config.label,
            'konvoy',
            execution,
            config.problemMatcher,
        );

        if (config.group) {
            task.group = config.group;
        }

        return task;
    }
}

export function registerTaskProvider(): vscode.Disposable {
    const provider = new KonvoyTaskProvider();

    const watcher = vscode.workspace.createFileSystemWatcher('**/konvoy.toml');
    watcher.onDidCreate(() => provider.invalidate());
    watcher.onDidDelete(() => provider.invalidate());

    const registration = vscode.tasks.registerTaskProvider('konvoy', provider);

    return {
        dispose: () => {
            registration.dispose();
            watcher.dispose();
        },
    };
}
