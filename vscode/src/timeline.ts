import * as vscode from 'vscode';

interface TraceRecord {
    pc: number;
    cycle: number;
    instruction: number;
    function?: string;
    registers: { [key: number]: number };
    memory_writes?: { address: number, old_value: number, new_value: number }[];
    stack_depth: number;
}

interface TraceResponse {
    traces: TraceRecord[];
    totalCycles: number;
}

export class TimelinePanel {
    private panel: vscode.WebviewPanel | undefined;
    private traces: TraceRecord[] = [];
    private totalCycles: number = 0;

    constructor(private context: vscode.ExtensionContext) { }

    public async show() {
        if (this.panel) {
            this.panel.reveal();
            return;
        }

        this.panel = vscode.window.createWebviewPanel(
            'labwiredTimeline',
            'LabWired Timeline',
            vscode.ViewColumn.Two,
            {
                enableScripts: true,
                retainContextWhenHidden: true,
            }
        );

        this.panel.webview.html = this.getWebviewContent();

        // Handle messages from webview
        this.panel.webview.onDidReceiveMessage(async (message) => {
            switch (message.command) {
                case 'jumpToCycle':
                    await this.jumpToCycle(message.cycle);
                    break;
                case 'refresh':
                    await this.refresh();
                    break;
            }
        });

        this.panel.onDidDispose(() => {
            this.panel = undefined;
        });

        await this.refresh();
    }

    private async refresh() {
        const session = vscode.debug.activeDebugSession;
        if (!session) return;

        try {
            const response: TraceResponse = await session.customRequest('readInstructionTrace', {
                startCycle: 0,
                endCycle: Number.MAX_SAFE_INTEGER,
            });

            this.traces = response.traces;
            this.totalCycles = response.totalCycles;

            this.panel?.webview.postMessage({
                command: 'updateTraces',
                traces: this.traces,
                totalCycles: this.totalCycles,
            });
        } catch (error) {
            console.error(`Failed to read instruction trace: ${error}`);
        }
    }

    private async jumpToCycle(cycle: number) {
        const session = vscode.debug.activeDebugSession;
        if (session) {
            // We use a custom request but could also use 'goto' if cycle-to-address mapping is known
            await session.customRequest('stepBack', { untilCycle: cycle });
        }
    }

    private getWebviewContent(): string {
        return `<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <style>
        body {
            margin: 0; padding: 0; overflow: hidden;
            background: var(--vscode-editor-background);
            color: var(--vscode-editor-foreground);
            font-family: var(--vscode-font-family);
        }
        #controls {
            padding: 12px; background: rgba(30, 30, 30, 0.5);
            backdrop-filter: blur(8px);
            border-bottom: 1px solid var(--vscode-panel-border);
            display: flex; gap: 15px; align-items: center;
        }
        button {
            background: var(--vscode-button-background);
            color: var(--vscode-button-foreground);
            border: none; padding: 6px 16px; cursor: pointer; border-radius: 4px;
            transition: background 0.2s;
        }
        button:hover { background: var(--vscode-button-hoverBackground); }
        #timeline-container { width: 100%; height: calc(100vh - 60px); position: relative; }
        #timeline { width: 100%; height: 100%; }
        #tooltip {
            position: absolute; background: var(--vscode-editorHoverWidget-background);
            border: 1px solid var(--vscode-editorHoverWidget-border);
            padding: 10px; border-radius: 6px; font-size: 12px;
            pointer-events: none; display: none; z-index: 1000;
            box-shadow: 0 4px 12px rgba(0,0,0,0.3);
        }
    </style>
</head>
<body>
    <div id="controls">
        <button id="refresh-btn">Refresh</button>
        <button id="reset-zoom-btn">Reset View</button>
        <span id="stats" style="font-size: 11px; opacity: 0.7"></span>
    </div>
    <div id="timeline-container">
        <canvas id="timeline"></canvas>
        <div id="tooltip"></div>
    </div>

    <script>
        const vscode = acquireVsCodeApi();
        const canvas = document.getElementById('timeline');
        const ctx = canvas.getContext('2d', { alpha: false });
        const tooltip = document.getElementById('tooltip');
        const stats = document.getElementById('stats');

        let traces = [];
        let zoomX = 10;
        let panX = 0;
        let isDragging = false;
        let dragStartX = 0;

        function resize() {
            canvas.width = window.innerWidth * window.devicePixelRatio;
            canvas.height = (window.innerHeight - 60) * window.devicePixelRatio;
            ctx.scale(window.devicePixelRatio, window.devicePixelRatio);
            render();
        }
        window.addEventListener('resize', resize);
        resize();

        function render() {
            const w = canvas.width / window.devicePixelRatio;
            const h = canvas.height / window.devicePixelRatio;
            ctx.fillStyle = getComputedStyle(document.body).getPropertyValue('--vscode-editor-background');
            ctx.fillRect(0, 0, w, h);

            if (!traces.length) return;

            const laneHeight = 20;
            const baseY = h - 40;

            // Draw Call Stack (Flame-graph style)
            traces.forEach((t, i) => {
                const x = (i * zoomX) + panX;
                if (x < -zoomX || x > w) return;

                const depth = t.stack_depth || 0;
                const top = baseY - (depth * laneHeight);

                // Function block
                ctx.fillStyle = '#1e88e5';
                ctx.globalAlpha = 0.6;
                ctx.fillRect(x, top, zoomX - 1, laneHeight);

                // Memory write indicator
                if (t.memory_writes && t.memory_writes.length > 0) {
                    ctx.fillStyle = '#ffb300';
                    ctx.globalAlpha = 1.0;
                    ctx.beginPath();
                    ctx.arc(x + zoomX/2, top - 4, 2, 0, Math.PI * 2);
                    ctx.fill();
                }
            });

            ctx.globalAlpha = 1.0;
            // Cycle markers
            ctx.fillStyle = '#888';
            ctx.font = '10px Inter, sans-serif';
            for (let i = 0; i < traces.length; i += Math.ceil(100 / zoomX)) {
                const x = (i * zoomX) + panX;
                if (x >= 0 && x <= w) {
                    ctx.fillText(traces[i].cycle, x, h - 5);
                }
            }
        }

        canvas.addEventListener('mousedown', e => { isDragging = true; dragStartX = e.clientX - panX; });
        window.addEventListener('mousemove', e => {
            if (isDragging) { panX = e.clientX - dragStartX; render(); }
            updateTooltip(e);
        });
        window.addEventListener('mouseup', () => isDragging = false);

        canvas.addEventListener('wheel', e => {
            e.preventDefault();
            const mouseX = e.clientX;
            const cycleAtMouse = (mouseX - panX) / zoomX;
            zoomX *= e.deltaY < 0 ? 1.2 : 0.8;
            zoomX = Math.max(0.01, Math.min(500, zoomX));
            panX = mouseX - (cycleAtMouse * zoomX);
            render();
        });

        function updateTooltip(e) {
            const x = e.clientX - panX;
            const idx = Math.floor(x / zoomX);
            if (idx >= 0 && idx < traces.length) {
                const t = traces[idx];
                tooltip.style.display = 'block';
                tooltip.style.left = (e.clientX + 15) + 'px';
                tooltip.style.top = (e.clientY + 15) + 'px';
                tooltip.innerHTML = \`
                    <div style="color: #64b5f6; font-weight: bold; margin-bottom: 4px">Cycle \${t.cycle}</div>
                    <div>PC: 0x\${t.pc.toString(16)}</div>
                    \${t.function ? \`<div>Func: \${t.function}</div>\` : ''}
                    <div>Stack Depth: \${t.stack_depth}</div>
                    \${t.memory_writes?.length ? \`<div style="color: #ffb300">Writes: \${t.memory_writes.length} bytes</div>\` : ''}
                \`;
            } else {
                tooltip.style.display = 'none';
            }
        }

        window.addEventListener('message', e => {
            if (e.data.command === 'updateTraces') {
                traces = e.data.traces;
                stats.textContent = \`\${traces.length} instructions | \${e.data.totalCycles} cycles\`;
                render();
            }
        });

        document.getElementById('refresh-btn').onclick = () => vscode.postMessage({ command: 'refresh' });
        document.getElementById('reset-zoom-btn').onclick = () => { zoomX = 10; panX = 0; render(); };
    </script>
</body>
</html>`;
    }
}
