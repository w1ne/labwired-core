// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

import * as vscode from 'vscode';
import * as path from 'path';
import { LabwiredDashboardProvider } from './dashboard';
import { TimelinePanel } from './timeline';

class LabwiredConfigurationProvider implements vscode.DebugConfigurationProvider {
    constructor(private readonly output: vscode.OutputChannel) { }

    async resolveDebugConfiguration(folder: vscode.WorkspaceFolder | undefined, config: vscode.DebugConfiguration, token?: vscode.CancellationToken): Promise<vscode.DebugConfiguration | undefined> {
        this.output.appendLine(`LabWired: Resolving debug configuration...`);

        if (!config.type && !config.request && !config.name) {
            // This is a "stub" config from an empty launch.json or no launch.json
            config.type = 'labwired';
            config.name = 'LabWired: Launch';
            config.request = 'launch';
            config.stopOnEntry = true;
        }

        if (!config.program) {
            this.output.appendLine("LabWired: Program not specified, attempting auto-detection...");
            const rootPath = folder?.uri.fsPath;
            if (rootPath) {
                const hasCargo = await fileExists(path.join(rootPath, 'Cargo.toml'));
                const hasMakefile = await fileExists(path.join(rootPath, 'Makefile'));

                if (hasCargo) {
                    config.program = path.join(rootPath, 'target', 'thumbv7m-none-eabi', 'debug', 'firmware');
                } else if (hasMakefile) {
                    config.program = path.join(rootPath, 'target', 'firmware');
                }
            }
        }

        if (!config.program || !await fileExists(config.program)) {
            this.output.appendLine("LabWired: ERROR - No program found to debug.");
            vscode.window.showErrorMessage("Cannot find a program to debug. Please ensure you have a Cargo.toml or Makefile and have built the project.");
            return undefined; // abort launch
        }

        // Auto-detect config files if they exist in root
        if (folder) {
            const rootPath = folder.uri.fsPath;
            if (!config.systemConfig) {
                const systemYaml = path.join(rootPath, 'system.yaml');
                if (await fileExists(systemYaml)) {
                    config.systemConfig = systemYaml;
                }
            }
            if (!config.mcuConfig) {
                const mcuYaml = path.join(rootPath, 'mcu.yaml');
                if (await fileExists(mcuYaml)) {
                    config.mcuConfig = mcuYaml;
                }
            }
        }

        this.output.appendLine(`LabWired: Launching ${config.program}`);
        return config;
    }
}

class LabwiredDebugAdapterDescriptorFactory implements vscode.DebugAdapterDescriptorFactory {
    constructor(private readonly output: vscode.OutputChannel) { }

    createDebugAdapterDescriptor(session: vscode.DebugSession, executable: vscode.DebugAdapterExecutable | undefined): vscode.ProviderResult<vscode.DebugAdapterDescriptor> {
        this.output.appendLine("LabWired: Creating Debug Adapter Descriptor...");

        // For development, use the absolute path we know works
        const dapPath = "/home/andrii/Projects/labwired/core/target/release/labwired-dap";
        this.output.appendLine(`LabWired: Using DAP binary at ${dapPath}`);

        return new vscode.DebugAdapterExecutable(dapPath, []);
    }
}

export function activate(context: vscode.ExtensionContext) {
    const outputChannel = vscode.window.createOutputChannel("LabWired");
    context.subscriptions.push(outputChannel);
    outputChannel.appendLine("LabWired Activated.");

    const dashboardProvider = new LabwiredDashboardProvider(context.extensionUri);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider(LabwiredDashboardProvider.viewType, dashboardProvider)
    );

    // Timeline Panel
    const timelinePanel = new TimelinePanel(context);
    context.subscriptions.push(
        vscode.commands.registerCommand('labwired.showTimeline', () => {
            timelinePanel.show();
        })
    );

    const factory = new LabwiredConfigurationProvider(outputChannel);
    context.subscriptions.push(vscode.debug.registerDebugConfigurationProvider('labwired', factory));

    const adapterFactory = new LabwiredDebugAdapterDescriptorFactory(outputChannel);
    context.subscriptions.push(vscode.debug.registerDebugAdapterDescriptorFactory('labwired', adapterFactory));

    // Handle Telemetry Events
    context.subscriptions.push(vscode.debug.onDidReceiveDebugSessionCustomEvent(e => {
        if (e.session.type === 'labwired') {
            if (e.event === 'telemetry') {
                dashboardProvider.updateTelemetry(e.body);
            }
        }
    }));

    // Handle standard output events for high-fidelity logging
    context.subscriptions.push(vscode.debug.onDidReceiveDebugSessionCustomEvent(e => {
        if (e.session.type === 'labwired' && e.event === 'output') {
            const body = e.body;
            if (body.category === 'stdout' || body.category === 'stderr') {
                outputChannel.appendLine(body.output);
            }
        }
    }));

    // Register Compile and Run command
    context.subscriptions.push(vscode.commands.registerCommand('labwired.compileAndRun', async () => {
        try {
            await compileAndRun(outputChannel);
        } catch (e) {
            vscode.window.showErrorMessage(`LabWired: Compile and Run failed: ${e}`);
            outputChannel.appendLine(`ERROR: ${e}`);
        }
    }));

    context.subscriptions.push(vscode.commands.registerCommand('labwired.showDashboard', () => {
        vscode.commands.executeCommand('labwired.dashboard.focus');
    }));

    // Status Bar Items
    const runBtn = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    runBtn.command = 'labwired.compileAndRun';
    runBtn.text = '$(play) Run in LabWired';
    runBtn.tooltip = 'Build firmware and launch LabWired simulation';
    runBtn.show();
    context.subscriptions.push(runBtn);

    const dashboardBtn = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 99);
    dashboardBtn.command = 'labwired.showDashboard';
    dashboardBtn.text = '$(dashboard) Dashboard';
    dashboardBtn.tooltip = 'Show LabWired Live Dashboard';
    dashboardBtn.show();
    context.subscriptions.push(dashboardBtn);
}

async function compileAndRun(outputChannel: vscode.OutputChannel) {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        throw new Error("No workspace folder open");
    }

    const rootPath = workspaceFolders[0].uri.fsPath;

    // 1. Detection
    const hasCargo = await fileExists(path.join(rootPath, 'Cargo.toml'));
    const hasMakefile = await fileExists(path.join(rootPath, 'Makefile'));

    let binaryPath: string;
    let buildCommand: string;
    let projectType: string;

    if (hasCargo) {
        projectType = 'Cargo';
        buildCommand = 'cargo build --target thumbv7m-none-eabi';
        binaryPath = path.join(rootPath, 'target', 'thumbv7m-none-eabi', 'debug', 'firmware');
    } else if (hasMakefile) {
        projectType = 'Makefile';
        buildCommand = 'make';
        binaryPath = path.join(rootPath, 'target', 'firmware');
    } else {
        throw new Error("Could not detect a supported project type (Cargo.toml or Makefile not found)");
    }

    // 2. Build
    outputChannel.show();
    outputChannel.appendLine(`LabWired: Building ${projectType} project in ${rootPath}...`);

    await vscode.window.withProgress({
        location: vscode.ProgressLocation.Notification,
        title: `LabWired: Building ${projectType} project...`,
        cancellable: false
    }, async (progress) => {
        return new Promise<void>((resolve, reject) => {
            const cp = require('child_process');
            cp.exec(buildCommand, { cwd: rootPath }, (err: any, stdout: string, stderr: string) => {
                outputChannel.append(stdout);
                outputChannel.append(stderr);
                if (err) {
                    reject(err);
                } else {
                    resolve();
                }
            });
        });
    });

    if (!await fileExists(binaryPath)) {
        throw new Error(`Could not find build artifact at ${binaryPath}. Ensure your build produces this file.`);
    }

    // 3. Start Debugging
    vscode.window.showInformationMessage(`LabWired: Launching simulation with ${path.basename(binaryPath)}...`);
    outputChannel.appendLine(`LabWired: Launching debug session for ${binaryPath}...`);

    const debugConfig: vscode.DebugConfiguration = {
        name: 'LabWired: Compile & Run',
        type: 'labwired',
        request: 'launch',
        program: binaryPath,
        stopOnEntry: true
    };

    // Auto-detect config files if they exist in root
    const systemYaml = path.join(rootPath, 'system.yaml');
    if (await fileExists(systemYaml)) {
        debugConfig.systemConfig = systemYaml;
        outputChannel.appendLine(`LabWired: Using system config: ${systemYaml}`);
    }
    const mcuYaml = path.join(rootPath, 'mcu.yaml');
    if (await fileExists(mcuYaml)) {
        debugConfig.mcuConfig = mcuYaml;
        outputChannel.appendLine(`LabWired: Using MCU config: ${mcuYaml}`);
    }

    const success = await vscode.debug.startDebugging(workspaceFolders[0], debugConfig);
    if (success) {
        outputChannel.appendLine("LabWired: Debug session started successfully.");
    } else {
        outputChannel.appendLine("LabWired: FAILED to start debug session.");
        vscode.window.showErrorMessage("LabWired: Debug session failed to start. Check LabWired Output for details.");
    }
}

async function fileExists(filePath: string): Promise<boolean> {
    try {
        await vscode.workspace.fs.stat(vscode.Uri.file(filePath));
        return true;
    } catch {
        return false;
    }
}

export function deactivate() {
}
