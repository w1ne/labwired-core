import * as vscode from 'vscode';

export class RTOSProvider implements vscode.TreeDataProvider<RTOSItem> {
    private _onDidChangeTreeData: vscode.EventEmitter<RTOSItem | undefined | void> = new vscode.EventEmitter<RTOSItem | undefined | void>();
    readonly onDidChangeTreeData: vscode.Event<RTOSItem | undefined | void> = this._onDidChangeTreeData.event;

    constructor() { }

    refresh(): void {
        this._onDidChangeTreeData.fire();
    }

    getTreeItem(element: RTOSItem): vscode.TreeItem {
        return element;
    }

    async getChildren(element?: RTOSItem, sessionOverride?: vscode.DebugSession): Promise<RTOSItem[]> {
        if (!element) {
            const session = sessionOverride || vscode.debug.activeDebugSession;
            if (!session) return [];

            try {
                // In a real RTOS, we'd read symbol tables or use a specific DAP request
                // For now, we mock it or use a custom 'readRTOSState' request
                const response = await session.customRequest('readRTOSState');
                return (response.tasks || []).map((t: any) => new RTOSItem(t.name, t.state, t.stackUsage, t.priority));
            } catch {
                return [];
            }
        }
        return [];
    }
}

class RTOSItem extends vscode.TreeItem {
    constructor(
        public readonly name: string,
        public readonly state: string,
        public readonly stackUsage: number,
        public readonly priority: number
    ) {
        super(name, vscode.TreeItemCollapsibleState.None);
        this.tooltip = `${name} (Priority: ${priority}, Stack: ${stackUsage}%)`;
        this.description = state;
        this.iconPath = this.getIconForState(state);
    }

    private getIconForState(state: string): vscode.ThemeIcon {
        switch (state.toLowerCase()) {
            case 'running': return new vscode.ThemeIcon('play-circle', new vscode.ThemeColor('debugIcon.startForeground'));
            case 'ready': return new vscode.ThemeIcon('circle-outline');
            case 'blocked': return new vscode.ThemeIcon('lock');
            case 'suspended': return new vscode.ThemeIcon('pause-circle');
            default: return new vscode.ThemeIcon('question');
        }
    }
}
