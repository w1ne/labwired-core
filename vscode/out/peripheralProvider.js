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
exports.PeripheralProvider = void 0;
const vscode = require("vscode");
class PeripheralProvider {
    constructor() {
        this._onDidChangeTreeData = new vscode.EventEmitter();
        this.onDidChangeTreeData = this._onDidChangeTreeData.event;
        this.peripherals = [];
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
                // Fetch peripherals from DAP session
                const session = sessionOverride || vscode.debug.activeDebugSession;
                if (!session)
                    return [];
                try {
                    const response = yield session.customRequest('readPeripherals');
                    this.peripherals = response.peripherals.map((p) => new PeripheralItem(p.name, p.base, p.size, p.registers));
                    return this.peripherals;
                }
                catch (_a) {
                    return [];
                }
            }
            return element.getChildren();
        });
    }
}
exports.PeripheralProvider = PeripheralProvider;
class PeripheralBaseItem extends vscode.TreeItem {
}
class PeripheralItem extends PeripheralBaseItem {
    constructor(name, base, size, registers) {
        super(name, vscode.TreeItemCollapsibleState.Collapsed);
        this.name = name;
        this.base = base;
        this.size = size;
        this.registers = registers;
        this.tooltip = `${name} @ 0x${base.toString(16)}`;
        this.description = `0x${base.toString(16)}`;
        this.iconPath = new vscode.ThemeIcon('chip');
    }
    getChildren() {
        return __awaiter(this, void 0, void 0, function* () {
            return this.registers.map(r => new RegisterItem(r.name, r.offset, r.value, r.fields));
        });
    }
}
class RegisterItem extends PeripheralBaseItem {
    constructor(name, offset, value, fields) {
        super(name, fields && fields.length > 0 ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None);
        this.name = name;
        this.offset = offset;
        this.value = value;
        this.fields = fields;
        this.tooltip = `${name} (+0x${offset.toString(16)}) = 0x${value.toString(16).padStart(8, '0')}`;
        this.description = `0x${value.toString(16).padStart(8, '0')}`;
        this.iconPath = new vscode.ThemeIcon('symbol-property');
    }
    getChildren() {
        return __awaiter(this, void 0, void 0, function* () {
            return (this.fields || []).map(f => new FieldItem(f.name, f.bitOffset, f.bitWidth, f.value, f.description));
        });
    }
}
class FieldItem extends PeripheralBaseItem {
    constructor(name, bitOffset, bitWidth, value, fieldDescription) {
        super(name, vscode.TreeItemCollapsibleState.None);
        this.name = name;
        this.bitOffset = bitOffset;
        this.bitWidth = bitWidth;
        this.value = value;
        this.fieldDescription = fieldDescription;
        this.tooltip = fieldDescription || `${name} [${bitOffset + bitWidth - 1}:${bitOffset}]`;
        this.description = `0x${value.toString(16)}`;
        this.iconPath = new vscode.ThemeIcon('symbol-field');
    }
    getChildren() {
        return __awaiter(this, void 0, void 0, function* () {
            return [];
        });
    }
}
//# sourceMappingURL=peripheralProvider.js.map
