// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

import * as vscode from 'vscode';
import * as path from 'path';

export async function showConfigWizard(): Promise<void> {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders) {
        vscode.window.showErrorMessage("Please open a workspace folder first.");
        return;
    }

    const rootPath = workspaceFolders[0].uri.fsPath;

    // 1. Select Chip
    const chips: vscode.QuickPickItem[] = [
        { label: '$(circuit-board) STM32F401', description: 'Cortex-M4, 84MHz', detail: 'Ideal for general-purpose embedded development.' },
        { label: '$(broadcast) nRF52832', description: 'Cortex-M4, BLE', detail: 'Perfect for Bluetooth Low Energy applications.' },
        { label: '$(microchip) RP2040', description: 'Dual-core Cortex-M0+, PIO', detail: 'Flexible I/O with Programmable I/O blocks.' },
        { label: '$(cpu) RISC-V Generic', description: '32-bit RISC-V Core', detail: 'Open-standard architecture simulation.' },
    ];

    const selectedChip = await vscode.window.showQuickPick(chips, {
        placeHolder: 'Select target MCU/Architecture',
        title: 'LabWired: Configuration Wizard (1/3)'
    });

    if (!selectedChip) return;

    // 2. Select Firmware
    // Attempt to find ELF files
    const elfFiles = await vscode.workspace.findFiles('**/target/**/debug/*', '**/node_modules/**');
    const elfItems = elfFiles.map(f => ({
        label: path.basename(f.fsPath),
        description: vscode.workspace.asRelativePath(f),
        uri: f
    }));

    let selectedElf;
    if (elfItems.length > 0) {
        selectedElf = await vscode.window.showQuickPick(elfItems, {
            placeHolder: 'Select firmware ELF file',
            title: 'LabWired: Configuration Wizard (2/3)'
        });
    }

    if (!selectedElf) {
        const manualPath = await vscode.window.showInputBox({
            prompt: 'Enter path to firmware ELF file (relative to root)',
            placeHolder: 'e.g. build/firmware.elf',
            title: 'LabWired: Configuration Wizard (2/3)'
        });
        if (!manualPath) return;
        selectedElf = { uri: vscode.Uri.file(path.join(rootPath, manualPath)) };
    }

    // 3. System Config (Optional)
    const hasSystemYaml = await fileExists(path.join(rootPath, 'system.yaml'));
    let useSystem = false;
    if (hasSystemYaml) {
        const choice = await vscode.window.showQuickPick(['Yes', 'No'], {
            placeHolder: 'Use existing system.yaml?',
            title: 'LabWired: Configuration Wizard (3/3)'
        });
        useSystem = choice === 'Yes';
    }

    // 4. Generate launch.json
    const launchConfig = {
        name: `LabWired: ${selectedChip.label}`,
        type: 'labwired',
        request: 'launch',
        program: vscode.workspace.asRelativePath(selectedElf.uri),
        system: useSystem ? 'system.yaml' : undefined,
        stopOnEntry: true
    };

    await updateLaunchConfig(workspaceFolders[0], launchConfig);
    vscode.window.showInformationMessage("LabWired: Configuration successful! Check .vscode/launch.json");

    // 5. Offer SVD Import (Asset Foundry)
    const importSvd = await vscode.window.showQuickPick(['Yes', 'No'], {
        placeHolder: 'Do you want to import chip peripherals (SVD) now?',
        title: 'LabWired: Asset Foundry Setup'
    });

    if (importSvd === 'Yes') {
        await importSvdWizard();
    }
}

async function updateLaunchConfig(folder: vscode.WorkspaceFolder, config: any): Promise<void> {
    const launchPath = path.join(folder.uri.fsPath, '.vscode', 'launch.json');
    const launchUri = vscode.Uri.file(launchPath);

    let launchJson: any = { version: "0.2.0", configurations: [] };
    try {
        const content = await vscode.workspace.fs.readFile(launchUri);
        launchJson = JSON.parse(content.toString());
    } catch {
        // file doesn't exist or is invalid
    }

    // Replace or add configuration
    const existingIndex = launchJson.configurations.findIndex((c: any) => c.name === config.name);
    if (existingIndex >= 0) {
        launchJson.configurations[existingIndex] = config;
    } else {
        launchJson.configurations.push(config);
    }

    const newContent = JSON.stringify(launchJson, null, 4);
    await vscode.workspace.fs.writeFile(launchUri, Buffer.from(newContent));
}

async function fileExists(filePath: string): Promise<boolean> {
    try {
        await vscode.workspace.fs.stat(vscode.Uri.file(filePath));
        return true;
    } catch {
        return false;
    }
}

export async function importSvdWizard(): Promise<void> {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders) {
        vscode.window.showErrorMessage("Please open a workspace folder first.");
        return;
    }
    const rootPath = workspaceFolders[0].uri.fsPath;

    // 1. Select SVD File
    const svdFiles = await vscode.workspace.findFiles('**/*.svd', '**/node_modules/**');
    let selectedSvd: vscode.Uri | undefined;

    if (svdFiles.length === 0) {
        const uris = await vscode.window.showOpenDialog({
            canSelectFiles: true,
            canSelectFolders: false,
            filters: { 'SVD Files': ['svd'] },
            title: 'Select SVD File'
        });
        if (uris && uris.length > 0) selectedSvd = uris[0];
    } else {
        const items = svdFiles.map(f => ({
            label: path.basename(f.fsPath),
            description: vscode.workspace.asRelativePath(f),
            uri: f
        }));
        items.push({ label: "$(file-directory) Browse...", description: "Select from disk", uri: vscode.Uri.file("") });

        const selection = await vscode.window.showQuickPick(items, {
            placeHolder: 'Select SVD file to import',
            title: 'LabWired: Import SVD (Asset Foundry)'
        });

        if (selection) {
            if (selection.label.startsWith("$(file-directory)")) {
                const uris = await vscode.window.showOpenDialog({
                    canSelectFiles: true,
                    filters: { 'SVD Files': ['svd'] }
                });
                if (uris && uris.length > 0) selectedSvd = uris[0];
            } else {
                selectedSvd = selection.uri;
            }
        }
    }

    if (!selectedSvd) return;

    // 2. Output Filename
    const outputName = await vscode.window.showInputBox({
        prompt: 'Enter output Rust filename',
        value: 'peripherals.rs',
        placeHolder: 'peripherals.rs'
    });

    if (!outputName) return;

    const cliPath = await getCliPath();
    if (!cliPath) {
        vscode.window.showErrorMessage("LabWired CLI not found. Please build the core project.");
        return;
    }

    // 3. Run Pipeline: SVD -> IR -> Rust
    const irPath = path.join(rootPath, 'labwired_ir.json');
    const rsPath = path.join(rootPath, outputName);

    await vscode.window.withProgress({
        location: vscode.ProgressLocation.Notification,
        title: "Asset Foundry: Generating Code...",
        cancellable: false
    }, async (progress) => {
        const cp = require('child_process');
        const exec = (cmd: string) => new Promise((res, rej) => {
            cp.exec(cmd, { cwd: rootPath }, (err: any, stdout: string, stderr: string) => {
                if (err) rej(stderr || stdout);
                else res(stdout);
            });
        });

        try {
            // Step A: SVD -> IR
            progress.report({ message: "Parsing SVD..." });
            await exec(`${cliPath} asset import-svd --input "${selectedSvd?.fsPath}" --output "${irPath}"`);

            // Step B: IR -> Rust
            progress.report({ message: `Generating ${outputName}...` });
            await exec(`${cliPath} asset codegen --input "${irPath}" --output "${rsPath}"`);

            vscode.window.showInformationMessage(`Asset Foundry: Successfully generated ${outputName}`);

            // Open the generated file
            const doc = await vscode.workspace.openTextDocument(rsPath);
            await vscode.window.showTextDocument(doc);

        } catch (e) {
            vscode.window.showErrorMessage(`Asset Foundry Failed: ${e}`);
        }
    });

    // 4. Update system.yaml (optional, if we want to auto-configure sim)
    // For now, focusing on the code generation part as requested.
}

async function getCliPath(): Promise<string | undefined> {
    // In dev env, default to one location
    const devPath = "/home/andrii/Projects/labwired/core/target/debug/labwired";
    if (await fileExists(devPath)) return devPath;

    const releasePath = "/home/andrii/Projects/labwired/core/target/release/labwired";
    if (await fileExists(releasePath)) return releasePath;

    // TODO: Add bundled path search for production extension
    return undefined;
}
