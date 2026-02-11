import * as vscode from 'vscode';

interface ProfileNode {
    name: string;
    value: number;
    children: ProfileNode[];
}

export class ProfilingPanel {
    private panel: vscode.WebviewPanel | undefined;

    constructor(private context: vscode.ExtensionContext) { }

    public async show() {
        if (this.panel) {
            this.panel.reveal();
            return;
        }

        this.panel = vscode.window.createWebviewPanel(
            'labwiredProfiling',
            'LabWired Performance Profiler',
            vscode.ViewColumn.Three,
            {
                enableScripts: true,
                retainContextWhenHidden: true,
            }
        );

        this.panel.webview.html = this.getWebviewContent();

        this.panel.webview.onDidReceiveMessage(async (message) => {
            switch (message.command) {
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

    public async refresh() {
        const session = vscode.debug.activeDebugSession;
        if (!session || !this.panel) return;

        try {
            const data: ProfileNode = await session.customRequest('readProfilingData');
            this.panel.webview.postMessage({
                command: 'updateProfiling',
                data: data
            });
        } catch (error) {
            console.error(`Failed to read profiling data: ${error}`);
        }
    }

    private getWebviewContent(): string {
        return `<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <style>
        body { font-family: 'Segoe UI', sans-serif; background: #1e1e1e; color: #d4d4d4; margin: 0; padding: 20px; }
        .flame-graph { width: 100%; height: 500px; }
        .node { stroke: #fff; stroke-width: 0.5px; cursor: pointer; }
        .node:hover { stroke: #000; stroke-width: 2px; }
        .label { font-size: 10px; fill: white; pointer-events: none; text-shadow: 0 1px 0 #000; }
        .tooltip { position: absolute; background: rgba(0,0,0,0.8); padding: 5px; border-radius: 4px; pointer-events: none; }
    </style>
</head>
<body>
    <h3>Flame Graph Profile</h3>
    <button onclick="refresh()">Refresh Profile</button>
    <div id="graph" class="flame-graph"></div>
    <div id="tooltip" class="tooltip" style="opacity: 0;"></div>

    <script src="https://d3js.org/d3.v7.min.js"></script>
    <script>
        const vscode = acquireVsCodeApi();
        const width = 800;
        const height = 400;

        window.addEventListener('message', event => {
            const message = event.data;
            if (message.command === 'updateProfiling') {
                render(message.data);
            }
        });

        function refresh() {
            vscode.postMessage({ command: 'refresh' });
        }

        function render(data) {
            const container = document.getElementById('graph');
            container.innerHTML = '';

            const svg = d3.select("#graph")
                .append("svg")
                .attr("width", "100%")
                .attr("height", "100%")
                .attr("viewBox", [0, 0, width, height]);

            const root = d3.hierarchy(data)
                .sum(d => d.value)
                .sort((a, b) => b.value - a.value);

            const partition = d3.partition()
                .size([width, (root.height + 1) * 25]);

            partition(root);

            const color = d3.scaleOrdinal(d3.quantize(d3.interpolateWarm, 10));

            const cell = svg.selectAll("g")
                .data(root.descendants())
                .join("g")
                .attr("transform", d => 'translate(' + d.x0 + ',' + d.y0 + ')');

            cell.append("rect")
                .attr("width", d => d.x1 - d.x0)
                .attr("height", d => d.y1 - d.y0)
                .attr("fill-opacity", 0.6)
                .attr("fill", d => color(d.data.name))
                .attr("class", "node")
                .on("mouseover", (event, d) => {
                    const tooltip = document.getElementById('tooltip');
                    tooltip.style.opacity = 1;
                    tooltip.innerHTML = '<strong>' + d.data.name + '</strong><br/>Cycles: ' + d.value;
                    tooltip.style.left = (event.pageX + 10) + 'px';
                    tooltip.style.top = (event.pageY + 10) + 'px';
                })
                .on("mouseout", () => {
                    document.getElementById('tooltip').style.opacity = 0;
                });

            cell.append("text")
                .filter(d => (d.x1 - d.x0) > 30)
                .attr("x", 4)
                .attr("y", 13)
                .attr("class", "label")
                .text(d => d.data.name);
        }
    </script>
</body>
</html>`;
    }
}
