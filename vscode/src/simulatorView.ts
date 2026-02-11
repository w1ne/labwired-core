// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

import * as vscode from 'vscode';
import { SimulatorManager, SimulatorStatus } from './simulatorManager';

export class SimulatorViewProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
    private _onDidChangeTreeData: vscode.EventEmitter<vscode.TreeItem | undefined | void> = new vscode.EventEmitter<vscode.TreeItem | undefined | void>();
    readonly onDidChangeTreeData: vscode.Event<vscode.TreeItem | undefined | void> = this._onDidChangeTreeData.event;

    constructor(private readonly manager: SimulatorManager) {
        this.manager.onStatusChanged(() => {
            this._onDidChangeTreeData.fire();
        });
    }

    getTreeItem(element: vscode.TreeItem): vscode.TreeItem {
        return element;
    }

    getChildren(element?: vscode.TreeItem): Thenable<vscode.TreeItem[]> {
        if (element) {
            return Promise.resolve([]);
        }

        const items: vscode.TreeItem[] = [];

        // 1. Session Info (Professional touch)
        const debugSession = vscode.debug.activeDebugSession;
        if (debugSession && debugSession.type === 'labwired') {
            const sessionGroup = new vscode.TreeItem('Current Session', vscode.TreeItemCollapsibleState.Expanded);
            sessionGroup.iconPath = new vscode.ThemeIcon('debug-alt');
            items.push(sessionGroup);

            const targetItem = new vscode.TreeItem('Target');
            targetItem.description = debugSession.configuration.name.replace('LabWired: ', '');
            targetItem.iconPath = new vscode.ThemeIcon('circuit-board');

            const firmwareItem = new vscode.TreeItem('Firmware');
            firmwareItem.description = vscode.workspace.asRelativePath(debugSession.configuration.program);
            firmwareItem.iconPath = new vscode.ThemeIcon('file-binary');

            // Return these as children of sessionGroup in a more complex impl,
            // but for simplicity we'll just add them to the flat list or use nesting.
            // Let's use simple flat list for now but grouped by labels.
            items.push(targetItem);
            items.push(firmwareItem);
        }

        // 2. Status Item
        const statusItem = new vscode.TreeItem('Simulator Status');
        statusItem.description = this.manager.status;
        statusItem.iconPath = this.getIconForStatus(this.manager.status);
        items.push(statusItem);

        // Actions
        if (this.manager.status === SimulatorStatus.Stopped || this.manager.status === SimulatorStatus.Error) {
            const startItem = new vscode.TreeItem('Start Simulator');
            startItem.command = {
                command: 'labwired.startSimulator',
                title: 'Start Simulator'
            };
            startItem.iconPath = new vscode.ThemeIcon('play');
            items.push(startItem);
        } else {
            const stopItem = new vscode.TreeItem('Stop Simulator');
            stopItem.command = {
                command: 'labwired.stopSimulator',
                title: 'Stop Simulator'
            };
            stopItem.iconPath = new vscode.ThemeIcon('stop');
            items.push(stopItem);

            const restartItem = new vscode.TreeItem('Restart Simulator');
            restartItem.command = {
                command: 'labwired.restartSimulator',
                title: 'Restart Simulator'
            };
            restartItem.iconPath = new vscode.ThemeIcon('refresh');
            items.push(restartItem);
        }

        const configItem = new vscode.TreeItem('Configure Project...');
        configItem.command = {
            command: 'labwired.configureProject',
            title: 'Configure Project'
        };
        configItem.iconPath = new vscode.ThemeIcon('gear');
        items.push(configItem);

        return Promise.resolve(items);
    }

    private getIconForStatus(status: SimulatorStatus): vscode.ThemeIcon {
        switch (status) {
            case SimulatorStatus.Running:
                return new vscode.ThemeIcon('pass-filled', new vscode.ThemeColor('testing.iconPassed'));
            case SimulatorStatus.Error:
                return new vscode.ThemeIcon('error', new vscode.ThemeColor('testing.iconFailed'));
            default:
                return new vscode.ThemeIcon('circle-outline');
        }
    }
}
