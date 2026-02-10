import * as vscode from 'vscode';

interface TraceRecord {
    pc: number;
    cycle: number;
    instruction: number;
    function?: string;
    registers: { [key: number]: number };
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

        // Handle panel disposal
        this.panel.onDidDispose(() => {
            this.panel = undefined;
        });

        // Initial load
        await this.refresh();
    }

    private async refresh() {
        // Send custom DAP request
        const session = vscode.debug.activeDebugSession;
        if (!session) {
            vscode.window.showWarningMessage('No active debug session. Start debugging first.');
            return;
        }

        try {
            const response: TraceResponse = await session.customRequest('readInstructionTrace', {
                startCycle: 0,
                endCycle: Number.MAX_SAFE_INTEGER,
            });

            this.traces = response.traces;
            this.totalCycles = response.totalCycles;

            // Send to webview
            this.panel?.webview.postMessage({
                command: 'updateTraces',
                traces: this.traces,
                totalCycles: this.totalCycles,
            });
        } catch (error) {
            vscode.window.showErrorMessage(`Failed to read instruction trace: ${error}`);
        }
    }

    private async jumpToCycle(cycle: number) {
        // TODO: Implement time-travel (restore state to specific cycle)
        vscode.window.showInformationMessage(`Jump to cycle ${cycle} (time-travel not yet implemented)`);
    }

    private getWebviewContent(): string {
        return `<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <style>
        body {
            margin: 0;
            padding: 0;
            overflow: hidden;
            background-color: var(--vscode-editor-background);
            color: var(--vscode-editor-foreground);
            font-family: var(--vscode-font-family);
        }

        #controls {
            padding: 10px;
            background-color: var(--vscode-editorWidget-background);
            border-bottom: 1px solid var(--vscode-editorWidget-border);
            display: flex;
            gap: 10px;
            align-items: center;
        }

        button {
            background-color: var(--vscode-button-background);
            color: var(--vscode-button-foreground);
            border: none;
            padding: 6px 14px;
            cursor: pointer;
            border-radius: 2px;
        }

        button:hover {
            background-color: var(--vscode-button-hoverBackground);
        }

        #stats {
            margin-left: auto;
            font-size: 12px;
            color: var(--vscode-descriptionForeground);
        }

        #timeline-container {
            width: 100%;
            height: calc(100vh - 50px);
            position: relative;
        }

        #timeline {
            width: 100%;
            height: 100%;
        }

        #tooltip {
            position: absolute;
            background-color: var(--vscode-editorHoverWidget-background);
            border: 1px solid var(--vscode-editorHoverWidget-border);
            padding: 8px;
            border-radius: 3px;
            font-size: 12px;
            pointer-events: none;
            display: none;
            z-index: 1000;
        }
    </style>
</head>
<body>
    <div id="controls">
        <button id="refresh-btn">Refresh</button>
        <button id="zoom-in-btn">Zoom In</button>
        <button id="zoom-out-btn">Zoom Out</button>
        <button id="reset-zoom-btn">Reset Zoom</button>
        <div id="stats">
            <span id="trace-count">0 instructions</span> |
            <span id="cycle-count">0 cycles</span>
        </div>
    </div>
    <div id="timeline-container">
        <canvas id="timeline"></canvas>
        <div id="tooltip"></div>
    </div>

    <script>
        const vscode = acquireVsCodeApi();
        const canvas = document.getElementById('timeline');
        const ctx = canvas.getContext('2d');
        const tooltip = document.getElementById('tooltip');
        const traceCountEl = document.getElementById('trace-count');
        const cycleCountEl = document.getElementById('cycle-count');

        let traces = [];
        let totalCycles = 0;
        let zoomLevel = 1.0;
        let panOffset = 0;
        let isDragging = false;
        let dragStart = 0;

        // Resize canvas to fill container
        function resize() {
            const container = document.getElementById('timeline-container');
            canvas.width = container.clientWidth;
            canvas.height = container.clientHeight;
            render();
        }
        window.addEventListener('resize', resize);
        resize();

        // Render timeline
        function render() {
            ctx.clearRect(0, 0, canvas.width, canvas.height);

            if (traces.length === 0) {
                ctx.fillStyle = '#888';
                ctx.font = '16px sans-serif';
                ctx.textAlign = 'center';
                ctx.fillText('No trace data. Start debugging and step through code...', canvas.width / 2, canvas.height / 2);
                return;
            }

            const width = canvas.width;
            const height = canvas.height;
            const step = (width / traces.length) * zoomLevel;

            // Draw instructions as vertical lines
            ctx.strokeStyle = '#0078d4';
            ctx.lineWidth = Math.max(1, step * 0.8);

            traces.forEach((trace, i) => {
                const x = (i * step) + panOffset;

                // Only draw if visible
                if (x < -step || x > width + step) return;

                const y = height / 2;
                const lineHeight = 20;

                // Color based on whether registers changed
                const hasRegChanges = Object.keys(trace.registers).length > 0;
                ctx.strokeStyle = hasRegChanges ? '#0078d4' : '#555';

                ctx.beginPath();
                ctx.moveTo(x, y - lineHeight);
                ctx.lineTo(x, y + lineHeight);
                ctx.stroke();
            });

            // Draw cycle markers
            ctx.fillStyle = '#888';
            ctx.font = '10px monospace';
            ctx.textAlign = 'center';
            const markerInterval = Math.max(1, Math.floor(traces.length / 10));
            for (let i = 0; i < traces.length; i += markerInterval) {
                const x = (i * step) + panOffset;
                if (x >= 0 && x <= width) {
                    ctx.fillText(\`\${traces[i].cycle}\`, x, 20);
                }
            }
        }

        // Handle click to jump to cycle
        canvas.addEventListener('click', (e) => {
            const rect = canvas.getBoundingClientRect();
            const x = e.clientX - rect.left - panOffset;
            const step = (canvas.width / traces.length) * zoomLevel;
            const index = Math.floor(x / step);

            if (index >= 0 && index < traces.length) {
                vscode.postMessage({
                    command: 'jumpToCycle',
                    cycle: traces[index].cycle,
                });
            }
        });

        // Handle mouse move for tooltip
        canvas.addEventListener('mousemove', (e) => {
            const rect = canvas.getBoundingClientRect();
            const x = e.clientX - rect.left - panOffset;
            const step = (canvas.width / traces.length) * zoomLevel;
            const index = Math.floor(x / step);

            if (index >= 0 && index < traces.length) {
                const trace = traces[index];
                const regChanges = Object.entries(trace.registers)
                    .map(([reg, val]) => \`R\${reg}=0x\${val.toString(16)}\`)
                    .join(', ');

                tooltip.innerHTML = \`
                    <strong>Cycle:</strong> \${trace.cycle}<br>
                    <strong>PC:</strong> 0x\${trace.pc.toString(16)}<br>
                    \${trace.function ? \`<strong>Function:</strong> \${trace.function}<br>\` : ''}
                    \${regChanges ? \`<strong>Changed:</strong> \${regChanges}\` : ''}
                \`;
                tooltip.style.left = e.clientX + 10 + 'px';
                tooltip.style.top = e.clientY + 10 + 'px';
                tooltip.style.display = 'block';
            } else {
                tooltip.style.display = 'none';
            }
        });

        canvas.addEventListener('mouseleave', () => {
            tooltip.style.display = 'none';
        });

        // Zoom controls
        canvas.addEventListener('wheel', (e) => {
            e.preventDefault();
            const zoomFactor = e.deltaY > 0 ? 0.9 : 1.1;
            zoomLevel *= zoomFactor;
            zoomLevel = Math.max(0.1, Math.min(100, zoomLevel));
            render();
        });

        // Pan controls
        canvas.addEventListener('mousedown', (e) => {
            isDragging = true;
            dragStart = e.offsetX - panOffset;
        });

        canvas.addEventListener('mousemove', (e) => {
            if (isDragging) {
                panOffset = e.offsetX - dragStart;
                render();
            }
        });

        canvas.addEventListener('mouseup', () => {
            isDragging = false;
        });

        canvas.addEventListener('mouseleave', () => {
            isDragging = false;
        });

        // Button handlers
        document.getElementById('refresh-btn').addEventListener('click', () => {
            vscode.postMessage({ command: 'refresh' });
        });

        document.getElementById('zoom-in-btn').addEventListener('click', () => {
            zoomLevel *= 1.2;
            render();
        });

        document.getElementById('zoom-out-btn').addEventListener('click', () => {
            zoomLevel *= 0.8;
            render();
        });

        document.getElementById('reset-zoom-btn').addEventListener('click', () => {
            zoomLevel = 1.0;
            panOffset = 0;
            render();
        });

        // Receive messages from extension
        window.addEventListener('message', (event) => {
            const message = event.data;
            if (message.command === 'updateTraces') {
                traces = message.traces;
                totalCycles = message.totalCycles;

                traceCountEl.textContent = \`\${traces.length} instructions\`;
                cycleCountEl.textContent = \`\${totalCycles} cycles\`;

                render();
            }
        });
    </script>
</body>
</html>`;
    }
}
