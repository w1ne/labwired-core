"use strict";
var __awaiter = (this && this.__awaiter) || function (thisArg, _arguments, P, generator) {
    function adopt(value) { return value instanceof P ? value : new P(function (resolve) { resolve(value); }); }
    return new (P || (P = Promise))(function (resolve, reject) {
        function fulfilled(value) { try { step(generator.next(value)); } catch (e) { reject(e); } }
        function rejected(value) { try { step(generator["throw"](value)); } catch (e) { reject(e); } }
        function step(result) { result.done ? resolve(result.value) : adopt(result.value).then(fulfilled, rejected); }
        step((generator = generator.apply(thisArg, _arguments || [])).next());
    });
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.DockerManager = void 0;
const cp = require("child_process");
class DockerManager {
    constructor(outputChannel) {
        this.outputChannel = outputChannel;
    }
    checkDockerAvailability() {
        return __awaiter(this, void 0, void 0, function* () {
            return new Promise((resolve) => {
                cp.exec('docker --version', (err) => {
                    resolve(!err);
                });
            });
        });
    }
    pullImage(imageName) {
        return __awaiter(this, void 0, void 0, function* () {
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
                    }
                    else {
                        reject(new Error(`Docker pull failed with code ${code}`));
                    }
                });
            });
        });
    }
    imageExists(imageName) {
        return __awaiter(this, void 0, void 0, function* () {
            return new Promise((resolve) => {
                cp.exec(`docker image inspect ${imageName}`, (err) => {
                    resolve(!err);
                });
            });
        });
    }
    getDockerRunCommand(imageName, workspacePath, args, command) {
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
    getDapArgs(imageName, workspacePath, extraDockerArgs = []) {
        // Mirror Mount: Map workspace to the SAME path inside container
        // This ensures that the file paths in the locally-built ELF match the container paths.
        const mount = `${workspacePath}:${workspacePath}`;
        return [
            'run',
            '-i',
            '--rm',
            '-v', mount,
            '-w', workspacePath,
            '-p', '9999:9999',
            ...extraDockerArgs,
            imageName,
            'labwired-dap' // Command to run
        ];
    }
}
exports.DockerManager = DockerManager;
//# sourceMappingURL=docker.js.map
