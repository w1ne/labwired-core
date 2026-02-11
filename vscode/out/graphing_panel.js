"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.GraphingPanel = void 0;
const vscode = require("vscode");
class GraphingPanel {
    constructor(context) {
        this.context = context;
        this.data = [];
    }
    show() {
        if (this.panel) {
            this.panel.reveal();
            return;
        }
        this.panel = vscode.window.createWebviewPanel('labwiredGraphing', 'LabWired Logic Analyzer', vscode.ViewColumn.Three, {
            enableScripts: true,
            retainContextWhenHidden: true,
        });
        this.panel.webview.html = this.getWebviewContent();
        this.panel.onDidDispose(() => {
            this.panel = undefined;
        });
        // Handle messages from the webview
        this.panel.webview.onDidReceiveMessage(message => {
            switch (message.command) {
                case 'ready':
                    // Send initial data if any
                    break;
            }
        });
    }
    updateSignals(signals) {
        var _a;
        (_a = this.panel) === null || _a === void 0 ? void 0 : _a.webview.postMessage({
            command: 'update',
            signals: signals
        });
    }
    getWebviewContent() {
        return `<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <style>
        body { margin: 0; padding: 0; background: #1e1e1e; color: #ccc; font-family: sans-serif; overflow: hidden; }
        canvas { width: 100%; height: 100vh; display: block; }
        #controls { position: absolute; top: 10px; left: 10px; background: rgba(0,0,0,0.5); padding: 5px; border-radius: 4px; pointer-events: none; }
    </style>
</head>
<body>
    <div id="controls">Logic Analyzer - Realtime</div>
    <canvas id="graph"></canvas>
    <script>
        const canvas = document.getElementById('graph');
        const ctx = canvas.getContext('2d');
        let signals = {};

        function resize() {
            canvas.width = window.innerWidth * window.devicePixelRatio;
            canvas.height = window.innerHeight * window.devicePixelRatio;
            ctx.scale(window.devicePixelRatio, window.devicePixelRatio);
        }
        window.addEventListener('resize', resize);
        resize();

        window.addEventListener('message', event => {
            const message = event.data;
            if (message.command === 'update') {
                signals = message.signals;
                render();
            }
        });

        function render() {
            const w = canvas.width / window.devicePixelRatio;
            const h = canvas.height / window.devicePixelRatio;
            ctx.clearRect(0, 0, w, h);

            const names = Object.keys(signals);
            if (names.length === 0) return;

            const laneH = h / names.length;

            names.forEach((name, i) => {
                const y = i * laneH;
                const data = signals[name];

                ctx.strokeStyle = '#444';
                ctx.strokeRect(0, y, w, laneH);
                ctx.fillStyle = '#aaa';
                ctx.fillText(name, 5, y + 15);

                ctx.strokeStyle = '#00ff00';
                ctx.beginPath();
                const step = w / data.length;
                data.forEach((v, j) => {
                    const vx = j * step;
                    const vy = y + laneH - (v * (laneH - 20)) - 10;
                    if (j === 0) ctx.moveTo(vx, vy);
                    else ctx.lineTo(vx, vy);
                });
                ctx.stroke();
            });
        }
    </script>
</body>
</html>`;
    }
}
exports.GraphingPanel = GraphingPanel;
//# sourceMappingURL=graphing_panel.js.map
