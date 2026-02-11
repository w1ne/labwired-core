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
exports.RTOSProvider = void 0;
const vscode = require("vscode");
class RTOSProvider {
    constructor() {
        this._onDidChangeTreeData = new vscode.EventEmitter();
        this.onDidChangeTreeData = this._onDidChangeTreeData.event;
    }
    refresh() {
        this._onDidChangeTreeData.fire();
    }
    getTreeItem(element) {
        return element;
    }
    getChildren(element, sessionOverride) {
        return __awaiter(this, void 0, void 0, function* () {
            if (!element) {
                const session = sessionOverride || vscode.debug.activeDebugSession;
                if (!session)
                    return [];
                try {
                    // In a real RTOS, we'd read symbol tables or use a specific DAP request
                    // For now, we mock it or use a custom 'readRTOSState' request
                    const response = yield session.customRequest('readRTOSState');
                    return (response.tasks || []).map((t) => new RTOSItem(t.name, t.state, t.stackUsage, t.priority));
                }
                catch (_a) {
                    return [];
                }
            }
            return [];
        });
    }
}
exports.RTOSProvider = RTOSProvider;
class RTOSItem extends vscode.TreeItem {
    constructor(name, state, stackUsage, priority) {
        super(name, vscode.TreeItemCollapsibleState.None);
        this.name = name;
        this.state = state;
        this.stackUsage = stackUsage;
        this.priority = priority;
        this.tooltip = `${name} (Priority: ${priority}, Stack: ${stackUsage}%)`;
        this.description = state;
        this.iconPath = this.getIconForState(state);
    }
    getIconForState(state) {
        switch (state.toLowerCase()) {
            case 'running': return new vscode.ThemeIcon('play-circle', new vscode.ThemeColor('debugIcon.startForeground'));
            case 'ready': return new vscode.ThemeIcon('circle-outline');
            case 'blocked': return new vscode.ThemeIcon('lock');
            case 'suspended': return new vscode.ThemeIcon('pause-circle');
            default: return new vscode.ThemeIcon('question');
        }
    }
}
//# sourceMappingURL=rtosProvider.js.map
