import * as cp from 'child_process';

export interface ILogOutput {
    append(value: string): void;
    appendLine(value: string): void;
}

export class DockerManager {
    constructor(private readonly outputChannel: ILogOutput) { }

    async checkDockerAvailability(): Promise<boolean> {
        return new Promise((resolve) => {
            cp.exec('docker --version', (err) => {
                resolve(!err);
            });
        });
    }

    async pullImage(imageName: string): Promise<void> {
        return new Promise((resolve, reject) => {
            this.outputChannel.appendLine(`LabWired: Pulling Docker image ${imageName}...`);
            const process = cp.spawn('docker', ['pull', imageName]);

            process.stdout.on('data', (data) => {
                this.outputChannel.append(data.toString());
            });

            process.stderr.on('data', (data) => {
                this.outputChannel.append(data.toString());
            });

            process.on('close', (code) => {
                if (code === 0) {
                    this.outputChannel.appendLine(`LabWired: Successfully pulled ${imageName}`);
                    resolve();
                } else {
                    reject(new Error(`Docker pull failed with code ${code}`));
                }
            });
        });
    }

    async imageExists(imageName: string): Promise<boolean> {
        return new Promise((resolve) => {
            cp.exec(`docker image inspect ${imageName}`, (err) => {
                resolve(!err);
            });
        });
    }

    getDockerRunCommand(imageName: string, workspacePath: string, args: string[], command: string): string {
        // We mount the workspace to the same path inside the container to preserve file paths for DAP
        const workDir = workspacePath;
        const mount = `${workspacePath}:${workspacePath}`;

        // Basic args: interactive, remove after exit, mount workspace, set workdir
        const dockerArgs = [
            'run', '-i', '--rm',
            '-v', mount,
            '-w', workDir,
            ...args,
            imageName,
            '/bin/sh', '-c', command
        ];

        return `docker ${dockerArgs.join(' ')}`;
    }

    getDapArgs(imageName: string, workspacePath: string, extraDockerArgs: string[] = []): string[] {
        // Mirror Mount: Map workspace to the SAME path inside container
        // This ensures that the file paths in the locally-built ELF match the container paths.
        const mount = `${workspacePath}:${workspacePath}`;

        return [
            'run',
            '-i',                 // Interactive (keep stdin open)
            '--rm',               // Remove container after exit
            '-v', mount,          // Mirror Mount
            '-w', workspacePath,  // Set working directory to workspace
            '-p', '9999:9999',    // Expose Dashboard port
            ...extraDockerArgs,   // User-defined args
            imageName,
            'labwired-dap'        // Command to run
        ];
    }
}
