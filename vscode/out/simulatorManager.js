"use strict";
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
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
exports.SimulatorManager = exports.SimulatorStatus = void 0;
const vscode = require("vscode");
var SimulatorStatus;
(function (SimulatorStatus) {
    SimulatorStatus["Stopped"] = "Stopped";
    SimulatorStatus["Running"] = "Running";
    SimulatorStatus["Error"] = "Error";
})(SimulatorStatus = exports.SimulatorStatus || (exports.SimulatorStatus = {}));
class SimulatorManager {
    constructor(output, cpModule = require('child_process')) {
        this.output = output;
        this.cpModule = cpModule;
        this._status = SimulatorStatus.Stopped;
        this._onStatusChanged = new vscode.EventEmitter();
        this.onStatusChanged = this._onStatusChanged.event;
    }
    get status() {
        return this._status;
    }
    set status(s) {
        this._status = s;
        this._onStatusChanged.fire(s);
    }
    start(binaryPath, args) {
        var _a, _b;
        return __awaiter(this, void 0, void 0, function* () {
            if (this.process) {
                this.output.appendLine("Simulator is already running.");
                return;
            }
            this.output.appendLine(`Starting simulator: ${binaryPath} ${args.join(' ')}`);
            try {
                this.process = this.cpModule.spawn(binaryPath, args);
                this.status = SimulatorStatus.Running;
                if (this.process) {
                    (_a = this.process.stdout) === null || _a === void 0 ? void 0 : _a.on('data', (data) => {
                        this.output.append(data.toString());
                    });
                    (_b = this.process.stderr) === null || _b === void 0 ? void 0 : _b.on('data', (data) => {
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
            }
            catch (e) {
                this.output.appendLine(`Failed to spawn simulator: ${e.message}`);
                this.status = SimulatorStatus.Error;
                throw e;
            }
        });
    }
    stop() {
        if (this.process) {
            this.output.appendLine("Stopping simulator...");
            this.process.kill();
            this.process = undefined;
            this.status = SimulatorStatus.Stopped;
        }
    }
    restart(binaryPath, args) {
        this.stop();
        return this.start(binaryPath, args);
    }
}
exports.SimulatorManager = SimulatorManager;
//# sourceMappingURL=simulatorManager.js.map
