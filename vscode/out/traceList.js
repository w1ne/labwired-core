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
exports.TraceListPanel = void 0;
const vscode = require("vscode");
class TraceListPanel {
    constructor(context) {
        this.context = context;
        this.traces = [];
    }
    show() {
        return __awaiter(this, void 0, void 0, function* () {
            if (this.panel) {
                this.panel.reveal();
                return;
            }
            this.panel = vscode.window.createWebviewPanel('labwiredTraceList', 'LabWired Instruction Trace', vscode.ViewColumn.Two, {
                enableScripts: true,
                retainContextWhenHidden: true,
            });
            this.panel.webview.html = this.getWebviewContent();
            this.panel.webview.onDidReceiveMessage((message) => __awaiter(this, void 0, void 0, function* () {
                switch (message.command) {
                    case 'jumpToCycle':
                        yield this.jumpToCycle(message.cycle);
                        break;
                    case 'refresh':
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
                const response = yield session.customRequest('readInstructionTrace', {
                    startCycle: 0,
                    endCycle: Number.MAX_SAFE_INTEGER
                });
                this.traces = response.traces;
                this.panel.webview.postMessage({
                    command: 'updateTraces',
                    traces: this.traces,
                    currentCycle: 0 // We'll simplify this for now
                });
            }
            catch (error) {
                console.error(`Failed to read instruction trace: ${error}`);
            }
        });
    }
    getCurrentCycle() {
        return __awaiter(this, void 0, void 0, function* () {
            // This is a bit of a hack since DAP doesn't have a 'getCurrentCycle' command
            // But we can infer it from the latest telemetry or just use the last cycle in the trace
            return 0; // Implementation-dependent
        });
    }
    jumpToCycle(cycle) {
        return __awaiter(this, void 0, void 0, function* () {
            const session = vscode.debug.activeDebugSession;
            if (session) {
                yield session.customRequest('stepBack', { untilCycle: cycle });
            }
        });
    }
    getWebviewContent() {
        return `<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <style>
        body { font-family: 'Segoe UI', sans-serif; background: #1e1e1e; color: #d4d4d4; margin: 0; padding: 0; }
        .toolbar { padding: 10px; border-bottom: 1px solid #333; position: sticky; top: 0; background: #1e1e1e; }
        .trace-table { width: 100%; border-collapse: collapse; font-family: 'Courier New', monospace; font-size: 13px; }
        .trace-table th { text-align: left; padding: 8px; border-bottom: 2px solid #333; color: #888; position: sticky; top: 40px; background: #1e1e1e; }
        .trace-table td { padding: 4px 8px; border-bottom: 1px solid #252525; }
        .row-even { background: #1e1e1e; }
        .row-odd { background: #252525; }
        .row-current { background: #2d4a63; color: white; }
        .cycle { color: #85c1e9; }
        .pc { color: #d7ba7d; cursor: pointer; }
        .pc:hover { text-decoration: underline; }
        .mnemonic { color: #ce9178; font-weight: bold; }
        .changes { color: #b5cea8; font-size: 11px; }
        .search-box { width: 200px; background: #333; color: white; border: 1px solid #555; padding: 4px; }
    </style>
</head>
<body>
    <div class="toolbar">
        Search: <input type="text" class="search-box" id="searchInput" oninput="filter()" placeholder="Mnemonic or PC...">
        <button onclick="refresh()">Refresh</button>
    </div>
    <div id="content">
        <table class="trace-table" id="traceTable">
            <thead>
                <tr>
                    <th>Cycle</th>
                    <th>PC</th>
                    <th>Instruction</th>
                    <th>Mnemonic</th>
                    <th>Changes</th>
                </tr>
            </thead>
            <tbody id="traceBody"></tbody>
        </table>
    </div>

    <script>
        const vscode = acquireVsCodeApi();
        let allTraces = [];

        window.addEventListener('message', event => {
            const message = event.data;
            if (message.command === 'updateTraces') {
                allTraces = message.traces;
                render(allTraces);
            }
        });

        function refresh() {
            vscode.postMessage({ command: 'refresh' });
        }

        function render(traces) {
            const body = document.getElementById('traceBody');
            body.innerHTML = traces.map((t, idx) => {
                let changes = [];
                for (const reg in t.registers) {
                    const [oldVal, newVal] = t.registers[reg];
                    changes.push('R' + reg + ':' + newVal.toString(16).toUpperCase());
                }
                if (t.memory_writes) {
                    t.memory_writes.forEach(w => {
                        changes.push('@' + w.address.toString(16).toUpperCase() + '=' + w.new_value.toString(16).toUpperCase());
                    });
                }

                return '<tr class="' + (idx % 2 === 0 ? 'row-even' : 'row-odd') + '" onclick="jumpToCycle(' + t.cycle + ')">' +
                    '<td class="cycle">' + t.cycle + '</td>' +
                    '<td class="pc">0x' + t.pc.toString(16).toUpperCase().padStart(8, '0') + '</td>' +
                    '<td>0x' + (t.instruction || 0).toString(16).toUpperCase().padStart(4, '0') + '</td>' +
                    '<td class="mnemonic">' + (t.mnemonic || '???') + '</td>' +
                    '<td class="changes">' + changes.join(', ') + '</td>' +
                    '</tr>';
            }).join('');
        }

        function filter() {
            const query = document.getElementById('searchInput').value.toLowerCase();
            const filtered = allTraces.filter(t =>
                (t.mnemonic && t.mnemonic.toLowerCase().includes(query)) ||
                t.pc.toString(16).toLowerCase().includes(query)
            );
            render(filtered);
        }

        function jumpToCycle(cycle) {
            vscode.postMessage({ command: 'jumpToCycle', cycle: cycle });
        }
    </script>
</body>
</html>`;
    }
}
exports.TraceListPanel = TraceListPanel;
//# sourceMappingURL=traceList.js.map
