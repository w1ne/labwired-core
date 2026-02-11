import * as vscode from 'vscode';
import * as path from 'path';

export class LabwiredDashboardProvider implements vscode.WebviewViewProvider {
	public static readonly viewType = 'labwired.dashboard';
	private _view?: vscode.WebviewView;

	constructor(
		private readonly _extensionUri: vscode.Uri,
	) { }

	public resolveWebviewView(
		webviewView: vscode.WebviewView,
		context: vscode.WebviewViewResolveContext,
		_token: vscode.CancellationToken,
	) {
		this._view = webviewView;

		webviewView.webview.options = {
			enableScripts: true,
			localResourceRoots: [
				this._extensionUri
			]
		};

		webviewView.webview.html = this._getHtmlForWebview(webviewView.webview);

		webviewView.webview.onDidReceiveMessage(data => {
			switch (data.type) {
				case 'colorSelected':
					{
						vscode.window.activeTextEditor?.insertSnippet(new vscode.SnippetString(`#${data.value}`));
						break;
					}
			}
		});
	}

	public updateTelemetry(data: any) {
		if (this._view) {
			this._view.webview.postMessage({ type: 'telemetry', data });
		}
	}

	private _getHtmlForWebview(webview: vscode.Webview) {
		const scriptUri = webview.asWebviewUri(vscode.Uri.joinPath(this._extensionUri, 'media', 'dashboard.js'));
		const styleUri = webview.asWebviewUri(vscode.Uri.joinPath(this._extensionUri, 'media', 'dashboard.css'));

		return `<!DOCTYPE html>
			<html lang="en">
			<head>
				<meta charset="UTF-8">
				<meta name="viewport" content="width=device-width, initial-scale=1.0">
				<link href="${styleUri}" rel="stylesheet">
				<title>LabWired Dashboard</title>
			</head>
			<body>
				<div class="container">
					<header>
						<h1>LabWired Live</h1>
						<div id="status-badge" class="badge">Connected</div>
					</header>

					<div class="grid">
						<div class="card">
							<h3>MIPS</h3>
							<div id="mips-value" class="value">0.00</div>
							<div class="sparkline-container">
								<canvas id="mips-sparkline" class="sparkline"></canvas>
							</div>
						</div>
						<div class="card">
							<h3>Cycles</h3>
							<div id="cycles-value" class="value">0</div>
							<div class="sparkline-container">
								<canvas id="cycles-sparkline" class="sparkline"></canvas>
							</div>
						</div>
						<div class="card">
							<h3>PC</h3>
							<div id="pc-value" class="value">0x00000000</div>
						</div>
						<div class="card">
							<h3>Status</h3>
							<div id="status-text" class="value">Ready</div>
						</div>
					</div>

					<div class="card full-width">
						<h3>Core Registers</h3>
						<div id="register-grid" class="reg-grid">
							<!-- Registers injected here -->
						</div>
					</div>

					<div class="log-container">
						<h3>System Log</h3>
						<div id="log-output" class="log"></div>
					</div>
				</div>
				<script src="${scriptUri}"></script>
			</body>
			</html>`;
	}
}
