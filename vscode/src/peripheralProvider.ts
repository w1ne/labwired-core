import * as vscode from 'vscode';

export class PeripheralProvider implements vscode.TreeDataProvider<PeripheralBaseItem> {
    private _onDidChangeTreeData: vscode.EventEmitter<PeripheralBaseItem | undefined | void> = new vscode.EventEmitter<PeripheralBaseItem | undefined | void>();
    readonly onDidChangeTreeData: vscode.Event<PeripheralBaseItem | undefined | void> = this._onDidChangeTreeData.event;

    private peripherals: PeripheralItem[] = [];

    constructor() { }

    refresh(): void {
        this._onDidChangeTreeData.fire();
    }

    getTreeItem(element: PeripheralBaseItem): vscode.TreeItem {
        return element;
    }

    async getChildren(element?: PeripheralBaseItem, sessionOverride?: vscode.DebugSession): Promise<PeripheralBaseItem[]> {
        if (!element) {
            // Fetch peripherals from DAP session
            const session = sessionOverride || vscode.debug.activeDebugSession;
            if (!session) return [];

            try {
                const response = await session.customRequest('readPeripherals');
                this.peripherals = response.peripherals.map((p: any) => new PeripheralItem(p.name, p.base, p.size, p.registers));
                return this.peripherals;
            } catch {
                return [];
            }
        }
        return element.getChildren();
    }
}

abstract class PeripheralBaseItem extends vscode.TreeItem {
    abstract getChildren(): Promise<PeripheralBaseItem[]>;
}

class PeripheralItem extends PeripheralBaseItem {
    constructor(
        public readonly name: string,
        public readonly base: number,
        public readonly size: number,
        private readonly registers: any[]
    ) {
        super(name, vscode.TreeItemCollapsibleState.Collapsed);
        this.tooltip = `${name} @ 0x${base.toString(16)}`;
        this.description = `0x${base.toString(16)}`;
        this.iconPath = new vscode.ThemeIcon('chip');
    }

    async getChildren(): Promise<PeripheralBaseItem[]> {
        return this.registers.map(r => new RegisterItem(r.name, r.offset, r.value, r.fields));
    }
}

class RegisterItem extends PeripheralBaseItem {
    constructor(
        public readonly name: string,
        public readonly offset: number,
        public readonly value: number,
        private readonly fields: any[]
    ) {
        super(name, fields && fields.length > 0 ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None);
        this.tooltip = `${name} (+0x${offset.toString(16)}) = 0x${value.toString(16).padStart(8, '0')}`;
        this.description = `0x${value.toString(16).padStart(8, '0')}`;
        this.iconPath = new vscode.ThemeIcon('symbol-property');
    }

    async getChildren(): Promise<PeripheralBaseItem[]> {
        return (this.fields || []).map(f => new FieldItem(f.name, f.bitOffset, f.bitWidth, f.value, f.description));
    }
}

class FieldItem extends PeripheralBaseItem {
    constructor(
        public readonly name: string,
        public readonly bitOffset: number,
        public readonly bitWidth: number,
        public readonly value: number,
        public readonly fieldDescription?: string
    ) {
        super(name, vscode.TreeItemCollapsibleState.None);
        this.tooltip = fieldDescription || `${name} [${bitOffset + bitWidth - 1}:${bitOffset}]`;
        this.description = `0x${value.toString(16)}`;
        this.iconPath = new vscode.ThemeIcon('symbol-field');
    }

    async getChildren(): Promise<PeripheralBaseItem[]> {
        return [];
    }
}
