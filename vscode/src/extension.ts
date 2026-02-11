// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

import * as vscode from 'vscode';
import * as path from 'path';
import { LabwiredDashboardProvider } from './dashboard';
import { TimelinePanel } from './timeline';
import { PeripheralProvider } from './peripheralProvider';
import { RTOSProvider } from './rtosProvider';
import { GraphingPanel } from './graphing_panel';
import { DockerManager } from './docker';
import { MemoryInspectorPanel } from './memoryInspector';
import { TraceListPanel } from './traceList';
import { ProfilingPanel } from './profilingPanel';
import { SimulatorManager } from './simulatorManager';
import { SimulatorViewProvider } from './simulatorView';
import { showConfigWizard } from './configWizard';


export class LabwiredConfigurationProvider implements vscode.DebugConfigurationProvider {
    constructor(
        private readonly output: vscode.OutputChannel,
        private readonly cpModule: any = require('child_process'),
        private readonly checkFile: (p: string) => Promise<boolean> = fileExists
    ) { }

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
                const hasCargo = await this.checkFile(path.join(rootPath, 'Cargo.toml'));
                const hasMakefile = await this.checkFile(path.join(rootPath, 'Makefile'));

                if (hasCargo) {
                    config.program = path.join(rootPath, 'target', 'thumbv7m-none-eabi', 'debug', 'firmware');
                } else if (hasMakefile) {
                    config.program = path.join(rootPath, 'target', 'firmware');
                }
            }
        }

        if (!config.program || !await this.checkFile(config.program)) {
            this.output.appendLine("LabWired: ERROR - No program found to debug.");
            vscode.window.showErrorMessage("Cannot find a program to debug. Please ensure you have a Cargo.toml or Makefile and have built the project.");
            return undefined; // abort launch
        }

        // Auto-detect config files if they exist in root
        if (folder) {
            const rootPath = folder.uri.fsPath;
            if (!config.systemConfig) {
                const systemYaml = path.join(rootPath, 'system.yaml');
                if (await this.checkFile(systemYaml)) {
                    config.systemConfig = systemYaml;
                }
            }
            if (!config.mcuConfig) {
                const mcuYaml = path.join(rootPath, 'mcu.yaml');
                if (await this.checkFile(mcuYaml)) {
                    config.mcuConfig = mcuYaml;
                }
            }
        }

        // Auto-detect Rust source for standard library to avoid "File not found" errors
        try {
            const sysroot = this.cpModule.execSync('rustc --print sysroot').toString().trim();
            const hashMatch = this.cpModule.execSync('rustc -vV').toString().match(/commit-hash:\s+([0-9a-f]+)/);

            if (hashMatch) {
                const hash = hashMatch[1];
                if (!config.sourceMap) {
                    config.sourceMap = {};
                }
                const rustSrcPath = path.join(sysroot, 'lib', 'rustlib', 'src', 'rust', 'library');
                config.sourceMap[`/rustc/${hash}/library`] = rustSrcPath;
                this.output.appendLine(`LabWired: Mapped Rust source ${hash} to ${rustSrcPath}`);
            }
        } catch (e) {
            // Silent catch to ensure debugger still starts even if rustc isn't in path
            this.output.appendLine(`LabWired: Could not auto-map Rust source: ${e}`);
        }

        this.output.appendLine(`LabWired: Launching ${config.program}`);
        return config;
    }
}

export class LabwiredDebugAdapterDescriptorFactory implements vscode.DebugAdapterDescriptorFactory {
    constructor(
        private readonly context: vscode.ExtensionContext,
        private readonly output: vscode.OutputChannel
    ) { }

    async createDebugAdapterDescriptor(session: vscode.DebugSession, executable: vscode.DebugAdapterExecutable | undefined): Promise<vscode.DebugAdapterDescriptor> {
        this.output.appendLine("LabWired: Creating Debug Adapter Descriptor...");

        // 1. Check User Config
        const config = vscode.workspace.getConfiguration('labwired');
        const executionMode = config.get<string>('executionMode') || 'local';

        if (executionMode === 'docker') {
            const dockerImage = config.get<string>('docker.image') || 'w1ne/labwired-dev:latest';
            const dockerArgs = config.get<string[]>('docker.runArgs') || [];
            const dockerManager = new DockerManager(this.output);

            const workspaceFolder = session.workspaceFolder;
            if (!workspaceFolder) {
                throw new Error("Debugging in Docker requires an open workspace.");
            }

            // Ensure image exists
            if (!await dockerManager.imageExists(dockerImage)) {
                const selection = await vscode.window.showErrorMessage(`Docker image '${dockerImage}' not found. Pull it now?`, 'Yes', 'No');
                if (selection === 'Yes') {
                    await dockerManager.pullImage(dockerImage);
                } else {
                    throw new Error("Docker image missing.");
                }
            }

            this.output.appendLine(`LabWired: Starting DAP in Docker container (${dockerImage})...`);

            const args = dockerManager.getDapArgs(dockerImage, workspaceFolder.uri.fsPath, dockerArgs);
            // The first arg is 'run', but DebugAdapterExecutable expects the command as first arg.
            // Actually, we want to run 'docker'.
            return new vscode.DebugAdapterExecutable('docker', args);
        }

        let dapPath = config.get<string>('dapPath');

        // 2. Check Bundled Binary (Local Mode)
        if (!dapPath) {
            const extPath = this.context.extensionUri.fsPath;
            // Determine platform extension
            const isWin = process.platform === 'win32';
            const binName = isWin ? 'labwired-dap.exe' : 'labwired-dap';

            // Check potential locations (dist/bin or bin)
            const bundledPath = path.join(extPath, 'dist', 'bin', binName);
            const devPath = path.join(extPath, 'bin', binName);

            if (await fileExists(bundledPath)) {
                dapPath = bundledPath;
            } else if (await fileExists(devPath)) {
                dapPath = devPath;
            } else {
                // FALLBACK for Dev Environment (hardcoded for now to keep existing flow working if binary not bundled yet)
                dapPath = "/home/andrii/Projects/labwired/core/target/release/labwired-dap";
            }
        }

        this.output.appendLine(`LabWired: Using DAP binary at ${dapPath}`);

        if (!await fileExists(dapPath)) {
            vscode.window.showErrorMessage(`LabWired: DAP binary not found at ${dapPath}. Please check your settings or reinstall the extension.`);
            throw new Error(`DAP binary not found at ${dapPath}`);
        }

        return new vscode.DebugAdapterExecutable(dapPath, []);
    }
}

export function activate(context: vscode.ExtensionContext) {
    const outputChannel = vscode.window.createOutputChannel("LabWired");
    context.subscriptions.push(outputChannel);
    outputChannel.appendLine("LabWired Activated.");

    // Simulator Management
    const simulatorManager = new SimulatorManager(outputChannel);
    const simulatorViewProvider = new SimulatorViewProvider(simulatorManager);
    context.subscriptions.push(
        vscode.window.registerTreeDataProvider('labwired.simulator', simulatorViewProvider)
    );

    context.subscriptions.push(vscode.commands.registerCommand('labwired.startSimulator', async () => {
        const workspaceFolders = vscode.workspace.workspaceFolders;
        if (!workspaceFolders) return;

        // Use default path or from settings
        const config = vscode.workspace.getConfiguration('labwired');
        const dapPath = config.get<string>('dapPath') || "/home/andrii/Projects/labwired/core/target/release/labwired-dap";

        // For simple start, we might need a program path.
        // We'll use the last active debug config or ask for one.
        const debugConfig = vscode.debug.activeDebugSession?.configuration;
        if (debugConfig && debugConfig.program) {
            await simulatorManager.start(dapPath, ["--gdb", "3333", "--firmware", debugConfig.program]);
        } else {
            vscode.window.showInformationMessage("Please start a debug session first to define the firmware, or use 'Compile and Run'.");
        }
    }));

    context.subscriptions.push(vscode.commands.registerCommand('labwired.stopSimulator', () => {
        simulatorManager.stop();
    }));

    context.subscriptions.push(vscode.commands.registerCommand('labwired.restartSimulator', async () => {
        const debugConfig = vscode.debug.activeDebugSession?.configuration;
        const config = vscode.workspace.getConfiguration('labwired');
        const dapPath = config.get<string>('dapPath') || "/home/andrii/Projects/labwired/core/target/release/labwired-dap";

        if (debugConfig && debugConfig.program) {
            await simulatorManager.restart(dapPath, ["--gdb", "3333", "--firmware", debugConfig.program]);
        }
    }));

    context.subscriptions.push(vscode.commands.registerCommand('labwired.configureProject', async () => {
        await showConfigWizard();
    }));

    context.subscriptions.push(vscode.commands.registerCommand('labwired.importSvd', async () => {
        // Dynamic import to avoid circular dependencies if any, though explicit import is better
        const { importSvdWizard } = require('./configWizard');
        await importSvdWizard();
    }));

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

    // Peripheral Tree View
    const peripheralProvider = new PeripheralProvider();
    context.subscriptions.push(
        vscode.window.registerTreeDataProvider('labwired.peripherals', peripheralProvider),
        vscode.commands.registerCommand('labwired.refreshPeripherals', () => peripheralProvider.refresh())
    );

    // RTOS Task View
    const rtosProvider = new RTOSProvider();
    context.subscriptions.push(
        vscode.window.registerTreeDataProvider('labwired.rtos_tasks', rtosProvider),
        vscode.commands.registerCommand('labwired.refreshRTOS', () => rtosProvider.refresh())
    );

    // Graphing Panel
    const graphingPanel = new GraphingPanel(context);
    context.subscriptions.push(
        vscode.commands.registerCommand('labwired.showGraphing', () => {
            graphingPanel.show();
        })
    );

    // Memory Inspector Panel
    const memoryInspectorPanel = new MemoryInspectorPanel(context);
    context.subscriptions.push(
        vscode.commands.registerCommand('labwired.showMemoryInspector', () => {
            memoryInspectorPanel.show();
        })
    );

    // Trace List Panel
    const traceListPanel = new TraceListPanel(context);
    context.subscriptions.push(
        vscode.commands.registerCommand('labwired.showTraceList', () => {
            traceListPanel.show();
        })
    );

    // Profiling Panel
    const profilingPanel = new ProfilingPanel(context);
    context.subscriptions.push(
        vscode.commands.registerCommand('labwired.showProfiling', () => {
            profilingPanel.show();
        })
    );

    const factory = new LabwiredConfigurationProvider(outputChannel);
    context.subscriptions.push(vscode.debug.registerDebugConfigurationProvider('labwired', factory));

    const adapterFactory = new LabwiredDebugAdapterDescriptorFactory(context, outputChannel);
    context.subscriptions.push(vscode.debug.registerDebugAdapterDescriptorFactory('labwired', adapterFactory));

    // Handle Telemetry and UI Sync
    context.subscriptions.push(vscode.debug.onDidReceiveDebugSessionCustomEvent(e => {
        if (e.session.type === 'labwired') {
            if (e.event === 'telemetry') {
                dashboardProvider.updateTelemetry(e.body);
                if (e.body.signals) {
                    graphingPanel.updateSignals(e.body.signals);
                }
            } else if (e.event === 'stopped' || e.event === 'continued') {
                peripheralProvider.refresh();
                rtosProvider.refresh();
                memoryInspectorPanel.refresh();
                traceListPanel.refresh();
                profilingPanel.refresh();
                // Optionally refresh timeline too if it's visible
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
            await compileAndRun(outputChannel, simulatorManager);
        } catch (e) {
            vscode.window.showErrorMessage(`LabWired: Compile and Run failed: ${e}`);
            outputChannel.appendLine(`ERROR: ${e}`);
        }
    }));



    // Check Docker Image on Startup if needed
    const config = vscode.workspace.getConfiguration('labwired');
    if (config.get<string>('executionMode') === 'docker' && config.get<boolean>('docker.autoPull')) {
        const image = config.get<string>('docker.image') || 'w1ne/labwired-dev:latest';
        const dockerManager = new DockerManager(outputChannel);
        dockerManager.imageExists(image).then(exists => {
            if (!exists) {
                vscode.window.withProgress({
                    location: vscode.ProgressLocation.Notification,
                    title: `LabWired: Pulling Docker image ${image}...`,
                    cancellable: false
                }, async () => {
                    try {
                        await dockerManager.pullImage(image);
                        vscode.window.showInformationMessage(`LabWired: Docker image ${image} ready.`);
                    } catch (e) {
                        vscode.window.showErrorMessage(`LabWired: Failed to pull Docker image: ${e}`);
                    }
                });
            }
        });
    }

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

async function compileAndRun(outputChannel: vscode.OutputChannel, simulatorManager: SimulatorManager) {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        throw new Error("No workspace folder open");
    }

    const rootPath = workspaceFolders[0].uri.fsPath;

    // 0. Stop simulator if running
    simulatorManager.stop();

    // 1. Detection
    const hasCargo = await fileExists(path.join(rootPath, 'Cargo.toml'));
    const hasMakefile = await fileExists(path.join(rootPath, 'Makefile'));

    let binaryPath: string;
    let buildCommand: string;
    let projectType: string;

    if (hasCargo) {
        projectType = 'Cargo';
        buildCommand = 'cargo build --target thumbv7m-none-eabi';

        // Dynamic detection of binary name
        let binName = 'firmware'; // Default
        try {
            const cargoContent = await vscode.workspace.fs.readFile(vscode.Uri.file(path.join(rootPath, 'Cargo.toml')));
            const cargoStr = Buffer.from(cargoContent).toString();

            // Simple regex to find [[bin]] name or [package] name
            const binMatch = cargoStr.match(/\[\[bin\]\]\s+name\s*=\s*"([^"]+)"/);
            const pkgMatch = cargoStr.match(/\[package\]\s+name\s*=\s*"([^"]+)"/);

            if (binMatch) {
                binName = binMatch[1];
            } else if (pkgMatch) {
                binName = pkgMatch[1];
            } else {
                // Fallback to folder name
                binName = path.basename(rootPath);
            }
        } catch (e) {
            binName = path.basename(rootPath);
        }

        // Detect target directory (workspace vs local)
        let targetDir = path.join(rootPath, 'target');
        const localTargetExists = await fileExists(path.join(targetDir, 'thumbv7m-none-eabi', 'debug', binName));

        if (!localTargetExists) {
            // Check one and two levels up for workspace target
            const parentTarget = path.join(rootPath, '..', 'target');
            const grandParentTarget = path.join(rootPath, '..', '..', 'target');

            if (await fileExists(path.join(parentTarget, 'thumbv7m-none-eabi', 'debug', binName))) {
                targetDir = parentTarget;
            } else if (await fileExists(path.join(grandParentTarget, 'thumbv7m-none-eabi', 'debug', binName))) {
                targetDir = grandParentTarget;
            }
        }

        binaryPath = path.join(targetDir, 'thumbv7m-none-eabi', 'debug', binName);
    } else if (hasMakefile) {
        projectType = 'Makefile';
        buildCommand = 'make';
        binaryPath = path.join(rootPath, 'target', 'firmware');
    } else {
        throw new Error("Could not detect a supported project type (Cargo.toml or Makefile not found)");
    }

    // 2. Build
    outputChannel.show();

    await vscode.window.withProgress({
        location: vscode.ProgressLocation.Notification,
        title: `LabWired: Building ${projectType} project...`,
        cancellable: false
    }, async (progress) => {
        progress.report({ message: "Running build command..." });
        return new Promise<void>((resolve, reject) => {
            const cp = require('child_process');
            cp.exec(buildCommand, { cwd: rootPath }, (err: any, stdout: string, stderr: string) => {
                outputChannel.append(stdout);
                outputChannel.append(stderr);
                if (err) {
                    reject(new Error(`Build failed. See LabWired output for details.`));
                } else {
                    resolve();
                }
            });
        });
    });

    if (!await fileExists(binaryPath)) {
        throw new Error(`Could not find build artifact at ${binaryPath}.`);
    }

    // 3. Start Simulator & Debugging
    vscode.window.showInformationMessage(`LabWired: Launching ${path.basename(binaryPath)}...`);

    // Start simulator in background via manager
    const config = vscode.workspace.getConfiguration('labwired');
    const dapPath = config.get<string>('dapPath') || "/home/andrii/Projects/labwired/core/target/release/labwired-dap";

    await simulatorManager.start(dapPath, ["--gdb", "3333", "--firmware", binaryPath]);

    const debugConfig: vscode.DebugConfiguration = {
        name: 'LabWired: Hot-Reload',
        type: 'labwired',
        request: 'launch',
        program: binaryPath,
        stopOnEntry: true
    };

    // Auto-detect config files
    const systemYaml = path.join(rootPath, 'system.yaml');
    if (await fileExists(systemYaml)) debugConfig.systemConfig = systemYaml;

    const mcuYaml = path.join(rootPath, 'mcu.yaml');
    if (await fileExists(mcuYaml)) debugConfig.mcuConfig = mcuYaml;

    await vscode.debug.startDebugging(workspaceFolders[0], debugConfig);
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
