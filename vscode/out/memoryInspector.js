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
exports.MemoryInspectorPanel = void 0;
const vscode = require("vscode");
class MemoryInspectorPanel {
    constructor(context) {
        this.context = context;
        this.currentAddress = 0x20000000; // Default to RAM
        this.currentCount = 256;
    }
    show() {
        return __awaiter(this, void 0, void 0, function* () {
            if (this.panel) {
                this.panel.reveal();
                return;
            }
            this.panel = vscode.window.createWebviewPanel('labwiredMemoryInspector', 'LabWired Memory Inspector', vscode.ViewColumn.Two, {
                enableScripts: true,
                retainContextWhenHidden: true,
            });
            this.panel.webview.html = this.getWebviewContent();
            this.panel.webview.onDidReceiveMessage((message) => __awaiter(this, void 0, void 0, function* () {
                switch (message.command) {
                    case 'readMemory':
                        this.currentAddress = message.address;
                        this.currentCount = message.count;
                        yield this.refresh();
                        break;
                    case 'writeMemory':
                        yield this.writeMemory(message.address, message.data);
                        yield this.refresh();
                        break;
                }
            }));
            this.panel.onDidDispose(() => {
                this.panel = undefined;
            });
            yield this.refresh();
        });
    }
    refresh() {
        return __awaiter(this, void 0, void 0, function* () {
            const session = vscode.debug.activeDebugSession;
            if (!session || !this.panel)
                return;
            try {
                const response = yield session.customRequest('readMemory', {
                    memoryReference: `0x${this.currentAddress.toString(16)}`,
                    offset: 0,
                    count: this.currentCount
                });
                this.panel.webview.postMessage({
                    command: 'updateMemory',
                    address: this.currentAddress,
                    data: response.data // base64
                });
            }
            catch (error) {
                console.error(`Failed to read memory: ${error}`);
            }
        });
    }
    writeMemory(address, hexData) {
        return __awaiter(this, void 0, void 0, function* () {
            const session = vscode.debug.activeDebugSession;
            if (!session)
                return;
            try {
                // Convert hex string to base64
                const buffer = Buffer.from(hexData, 'hex');
                const data64 = buffer.toString('base64');
                yield session.customRequest('writeMemory', {
                    memoryReference: `0x${address.toString(16)}`,
                    offset: 0,
                    data: data64
                });
            }
            catch (error) {
                vscode.window.showErrorMessage(`Failed to write memory: ${error}`);
            }
        });
    }
    getWebviewContent() {
        return `<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <style>
        body { font-family: 'Courier New', Courier, monospace; background: #1e1e1e; color: #d4d4d4; }
        .toolbar { padding: 10px; border-bottom: 1px solid #333; display: flex; gap: 10px; }
        input { background: #333; color: white; border: 1px solid #555; padding: 4px; }
        .hex-table { width: 100%; border-collapse: collapse; margin-top: 10px; font-size: 14px; }
        .hex-table th, .hex-table td { padding: 4px 8px; border: 1px solid #333; }
        .address { color: #85c1e9; }
        .byte { cursor: pointer; }
        .byte:hover { background: #444; }
        .byte.editing { background: #2e86c1; color: white; }
        .ascii { color: #52be80; border-left: 2px solid #555 !important; }
    </style>
</head>
<body>
    <div class="toolbar">
        Address: <input type="text" id="addressInput" value="0x20000000">
        Size: <input type="number" id="sizeInput" value="256" step="16">
        <button onclick="refresh()">Go</button>
    </div>
    <div id="content"></div>

    <script>
        const vscode = acquireVsCodeApi();
        let currentData = null;
        let baseAddress = 0;

        window.addEventListener('message', event => {
            const message = event.data;
            if (message.command === 'updateMemory') {
                baseAddress = message.address;
                render(message.data);
            }
        });

        function refresh() {
            const addr = parseInt(document.getElementById('addressInput').value);
            const size = parseInt(document.getElementById('sizeInput').value);
            vscode.postMessage({ command: 'readMemory', address: addr, count: size });
        }

        function render(base64Data) {
            const binary = atob(base64Data);
            const bytes = new Uint8Array(binary.length);
            for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);

            let html = '<table class="hex-table"><tr><th>Address</th>';
            for (let i = 0; i < 16; i++) html += '<th>' + i.toString(16).toUpperCase().padStart(2, '0') + '</th>';
            html += '<th class="ascii">ASCII</th></tr>';

            for (let row = 0; row < bytes.length; row += 16) {
                const rowAddr = baseAddress + row;
                html += '<tr><td class="address">0x' + rowAddr.toString(16).toUpperCase().padStart(8, '0') + '</td>';

                let ascii = '';
                for (let col = 0; col < 16; col++) {
                    const idx = row + col;
                    if (idx < bytes.length) {
                        const b = bytes[idx];
                        html += '<td class="byte" onclick="editByte(' + idx + ', ' + b + ')">' + b.toString(16).toUpperCase().padStart(2, '0') + '</td>';
                        ascii += (b >= 32 && b <= 126) ? String.fromCharCode(b) : '.';
                    } else {
                        html += '<td></td>';
                    }
                }
                html += '<td class="ascii">' + escapeHtml(ascii) + '</td></tr>';
            }
            html += '</table>';
            document.getElementById('content').innerHTML = html;
        }

        function editByte(idx, currentVal) {
            const newVal = prompt('Enter new hex value for byte at offset ' + idx + ':', currentVal.toString(16).toUpperCase());
            if (newVal !== null) {
                const addr = baseAddress + idx;
                vscode.postMessage({ command: 'writeMemory', address: addr, data: newVal.padStart(2, '0') });
            }
        }

        function escapeHtml(text) {
            return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
        }
    </script>
</body>
</html>`;
    }
}
exports.MemoryInspectorPanel = MemoryInspectorPanel;
//# sourceMappingURL=memoryInspector.js.map
