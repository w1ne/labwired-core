// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

import * as vscode from 'vscode';
import * as child_process from 'child_process';

export enum SimulatorStatus {
    Stopped = 'Stopped',
    Running = 'Running',
    Error = 'Error'
}

export class SimulatorManager {
    private process: child_process.ChildProcess | undefined;
    private _status: SimulatorStatus = SimulatorStatus.Stopped;
    private _onStatusChanged = new vscode.EventEmitter<SimulatorStatus>();
    public readonly onStatusChanged = this._onStatusChanged.event;

    constructor(
        private readonly output: vscode.OutputChannel,
        private readonly cpModule: any = require('child_process')
    ) { }

    get status(): SimulatorStatus {
        return this._status;
    }

    private set status(s: SimulatorStatus) {
        this._status = s;
        this._onStatusChanged.fire(s);
    }

    public async start(binaryPath: string, args: string[]): Promise<void> {
        if (this.process) {
            this.output.appendLine("Simulator is already running.");
            return;
        }

        this.output.appendLine(`Starting simulator: ${binaryPath} ${args.join(' ')}`);

        try {
            this.process = this.cpModule.spawn(binaryPath, args);
            this.status = SimulatorStatus.Running;

            if (this.process) {
                this.process.stdout?.on('data', (data) => {
                    this.output.append(data.toString());
                });

                this.process.stderr?.on('data', (data) => {
                    this.output.append(data.toString());
                });

                this.process.on('close', (code) => {
                    this.output.appendLine(`Simulator process exited with code ${code}`);
                    this.process = undefined;
                    this.status = code === 0 ? SimulatorStatus.Stopped : SimulatorStatus.Error;
                });

                this.process.on('error', (err) => {
                    this.output.appendLine(`Simulator process error: ${err.message}`);
                    this.status = SimulatorStatus.Error;
                });
            }
        } catch (e: any) {
            this.output.appendLine(`Failed to spawn simulator: ${e.message}`);
            this.status = SimulatorStatus.Error;
            throw e;
        }
    }

    public stop(): void {
        if (this.process) {
            this.output.appendLine("Stopping simulator...");
            this.process.kill();
            this.process = undefined;
            this.status = SimulatorStatus.Stopped;
        }
    }

    public restart(binaryPath: string, args: string[]): Promise<void> {
        this.stop();
        return this.start(binaryPath, args);
    }
}
