export const AGENT_HARDWARE_LOOP_URI = 'labwired://guides/agent-hardware-loop';
export const AGENT_HARDWARE_LOOP_NAME = 'labwired-agent-hardware-loop';
export const AGENT_HARDWARE_LOOP_MIME = 'text/markdown';
export const HARDWARE_LAB_TEMPLATE_URI = 'ui://widget/labwired-hardware-lab-v8.html';
export const HARDWARE_LAB_TEMPLATE_NAME = 'labwired-hardware-lab';
export const HARDWARE_LAB_TEMPLATE_MIME = 'text/html;profile=mcp-app';
export const HARDWARE_LAB_TEMPLATE_ALIASES = [
  'ui://labwired/hardware-lab.html',
  'ui://labwired/hardware-lab-v2.html',
  'ui://labwired/hardware-lab-v3.html',
  'ui://labwired/hardware-lab-v4.html',
  'ui://labwired/hardware-lab-v5.html',
  'ui://widget/hardware-lab-v6.html',
  'ui://widget/labwired-hardware-lab-v6.html',
  'ui://widget/hardware-lab-v7.html',
  'ui://widget/labwired-hardware-lab-v7.html',
  'ui://widget/hardware-lab-v8.html',
  'ui://widget/labwired-hardware-lab-v8.html',
] as const;

export const HARDWARE_LAB_WIDGET_CSP = {
  connectDomains: ['https://app.labwired.com', 'https://api.labwired.com'],
  frameDomains: ['https://app.labwired.com'],
  resourceDomains: ['https://labwired.com', 'https://app.labwired.com'],
};

export const HARDWARE_LAB_OPENAI_WIDGET_CSP = {
  connect_domains: HARDWARE_LAB_WIDGET_CSP.connectDomains,
  frame_domains: HARDWARE_LAB_WIDGET_CSP.frameDomains,
  resource_domains: HARDWARE_LAB_WIDGET_CSP.resourceDomains,
  redirect_domains: ['https://app.labwired.com'],
};

const AGENT_HARDWARE_LOOP_TEXT = `# LabWired agent hardware loop

Use LabWired as a deterministic virtual hardware lab for firmware work.

1. Choose a board with \`labwired_list_boards\`.
2. Discover modeled peripherals with \`labwired_list_components\`.
3. Build or update the diagram with an MCU, components, and wires.
4. Use the canonical LabWiredDiagramV1 shape for outbound Playground links: \`version\`, \`board\`, editor-safe \`parts[].attrs/x/y/rotate\`, and \`wires[].color\`. Compact input is accepted, but MCP responses normalize before sharing.
5. Validate the diagram with \`labwired_validate_diagram\` before running.
6. Compile firmware outside hosted MCP using the documented scaffold and target flags.
7. Run the ELF with \`labwired_run\`, passing \`elf_base64\`, \`target\`, and \`diagram\`. The hosted connector compiles the diagram internally — pass the diagram object directly, not a system YAML. Use \`labwired_compile_diagram\` separately if you need to inspect or download the compiled \`system_yaml\`.
8. Inspect serial output, cycle counts, stop reasons, and hardware diagnosis.
9. Iterate on firmware or wiring until simulator evidence matches the intended behavior.

Hosted MCP accepts Clerk OAuth bearer tokens and \`lwk_live_\` workspace API keys. The hosted connector runs firmware through the LabWired builder; it does not compile source.
Viewing uses the real Playground: inline ChatGPT views embed \`https://app.labwired.com/?embed=true#...\`; full sessions open \`https://app.labwired.com/#...\`.
`;

const HARDWARE_LAB_TEMPLATE_TEXT = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>LabWired Hardware Lab</title>
    <style>
      :root { color-scheme: light dark; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
      body { margin: 0; background: #0f172a; color: #e5e7eb; }
      main { display: grid; grid-template-rows: auto minmax(620px, 1fr) auto; gap: 10px; min-height: 100vh; padding: 10px; box-sizing: border-box; }
      header { display: flex; justify-content: space-between; gap: 12px; align-items: center; }
      .actions { display: flex; align-items: center; gap: 10px; flex-shrink: 0; }
      h1 { font-size: 18px; margin: 0; font-weight: 650; }
      a { color: #67e8f9; }
      button.fs { appearance: none; cursor: pointer; display: inline-flex; align-items: center; gap: 6px; font: inherit; font-size: 13px; color: #e5e7eb; background: #1e293b; border: 1px solid #334155; border-radius: 7px; padding: 6px 11px; }
      button.fs:hover { background: #273449; border-color: #475569; }
      button.fs[hidden] { display: none; }
      .frame-shell { min-height: 620px; border: 1px solid #334155; border-radius: 8px; background: #020617; overflow: hidden; position: relative; }
      iframe { width: 100%; height: 100%; min-height: 620px; border: 0; display: block; background: #020617; }
      .fallback { position: absolute; inset: 0; display: grid; place-items: center; padding: 18px; text-align: center; background: #020617; box-sizing: border-box; }
      .fallback[hidden] { display: none; }
      .muted { color: #94a3b8; font-size: 12px; }
    </style>
  </head>
  <body>
    <main>
      <header>
        <div>
          <h1>LabWired Hardware Lab</h1>
          <div class="muted">Embedded hardware view for agent-generated diagrams.</div>
        </div>
        <div class="actions">
          <button type="button" id="fullscreen" class="fs" hidden aria-label="Expand the hardware lab to full screen">
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 2H2v4M10 2h4v4M6 14H2v-4M10 14h4v-4"/></svg>
            <span id="fullscreen-label">Full screen</span>
          </button>
          <a id="watch" target="_blank" rel="noreferrer" aria-disabled="true">Waiting for lab link</a>
        </div>
      </header>
      <section class="frame-shell" aria-label="LabWired embedded hardware lab">
        <iframe id="labwired-frame" title="LabWired Playground" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox"></iframe>
        <div id="fallback" class="fallback">
          <div>
            <strong>Waiting for LabWired Playground</strong>
            <div class="muted">The agent tool result will provide the embedded project URL.</div>
          </div>
        </div>
      </section>
    </main>
    <script>
      const frame = document.getElementById('labwired-frame');
      const fallback = document.getElementById('fallback');
      const watch = document.getElementById('watch');
      const fullscreen = document.getElementById('fullscreen');
      const fullscreenLabel = document.getElementById('fullscreen-label');
      // Display-mode control: in a constrained inline pane the board is small and
      // pan/drag fights the chat scroll, so offer a one-tap expand to the full
      // host pane. Only shown when the Apps SDK host exposes requestDisplayMode.
      function syncDisplayMode() {
        const mode = window.openai && window.openai.displayMode;
        const isFull = mode === 'fullscreen' || mode === 'pip';
        if (fullscreenLabel) fullscreenLabel.textContent = isFull ? 'Exit full screen' : 'Full screen';
      }
      function setupFullscreen() {
        if (!fullscreen) return;
        if (!window.openai || typeof window.openai.requestDisplayMode !== 'function') {
          fullscreen.hidden = true;
          return;
        }
        fullscreen.hidden = false;
        syncDisplayMode();
        fullscreen.addEventListener('click', async () => {
          const mode = window.openai && window.openai.displayMode;
          const next = mode === 'fullscreen' || mode === 'pip' ? 'inline' : 'fullscreen';
          try { await window.openai.requestDisplayMode({ mode: next }); } catch (err) { void err; }
          syncDisplayMode();
        });
      }
      setupFullscreen();
      function toolData(value) {
        if (!value || typeof value !== 'object') return {};
        if (value.result && typeof value.result === 'object') return toolData(value.result);
        if (value.structuredContent && typeof value.structuredContent === 'object') return value.structuredContent;
        if (value.toolOutput && typeof value.toolOutput === 'object') return value.toolOutput;
        if (value.mcp_tool_result && typeof value.mcp_tool_result === 'object') return toolData(value.mcp_tool_result);
        if (value.call_tool_result && typeof value.call_tool_result === 'object') return toolData(value.call_tool_result);
        return value;
      }
      function currentBridgeData() {
        return toolData(
          window.openai?.toolOutput ??
          window.openai?.structuredContent ??
          window.openai?.toolResponseMetadata
        );
      }
      function render(value) {
        const data = toolData(value);
        const frameUrl = data.inline_frame_url ?? data.studio_url ?? data.share_url ?? '';
        if (frameUrl) {
          if (frame.src !== frameUrl) frame.src = frameUrl;
          fallback.hidden = true;
          watch.href = data.studio_url ?? frameUrl;
          watch.textContent = 'Open in LabWired Studio';
          watch.setAttribute('aria-disabled', 'false');
          if (window.openai?.setOpenInAppUrl) {
            window.openai.setOpenInAppUrl({ href: data.studio_url ?? frameUrl });
          }
        } else {
          frame.removeAttribute('src');
          fallback.hidden = false;
          watch.removeAttribute('href');
          watch.textContent = 'Waiting for lab link';
          watch.setAttribute('aria-disabled', 'true');
        }
      }
      render(currentBridgeData());
      watch.addEventListener('click', (event) => {
        if (!watch.href) return;
        if (window.openai?.openExternal) {
          event.preventDefault();
          window.openai.openExternal({ href: watch.href, redirectUrl: false });
        }
      });
      window.addEventListener('message', (event) => {
        const message = event.data;
        if (!message || typeof message !== 'object') return;
        if (message.method === 'ui/notifications/tool-result') {
          render(message.params);
        }
      });
      window.addEventListener('openai:set_globals', () => { render(currentBridgeData()); syncDisplayMode(); });
    </script>
  </body>
</html>`;

export interface McpResourceDescriptor {
  uri: string;
  name: string;
  mimeType: string;
  description: string;
  _meta?: Record<string, unknown>;
}

export interface McpResourceContent {
  uri: string;
  mimeType: string;
  text: string;
  _meta?: Record<string, unknown>;
}

export function hardwareLabComponentMeta(): Record<string, unknown> {
  return {
    ui: { prefersBorder: true, csp: HARDWARE_LAB_WIDGET_CSP },
    'openai/widgetDescription': 'Interactive LabWired hardware board, firmware state, and simulator evidence.',
    'openai/widgetCSP': HARDWARE_LAB_OPENAI_WIDGET_CSP,
    'openai/widgetPrefersBorder': true,
  };
}

export const RESOURCES: readonly McpResourceDescriptor[] = [
  {
    uri: AGENT_HARDWARE_LOOP_URI,
    name: AGENT_HARDWARE_LOOP_NAME,
    mimeType: AGENT_HARDWARE_LOOP_MIME,
    description: 'Guide for using LabWired as an agent-driven virtual hardware lab.',
  },
  {
    uri: HARDWARE_LAB_TEMPLATE_URI,
    name: HARDWARE_LAB_TEMPLATE_NAME,
    mimeType: HARDWARE_LAB_TEMPLATE_MIME,
    description: 'Embeddable hardware lab UI for ChatGPT/Claude-capable MCP clients.',
    _meta: hardwareLabComponentMeta(),
  },
];

export function getResource(uri: string): McpResourceContent | null {
  if (uri === AGENT_HARDWARE_LOOP_URI) {
    return {
      uri,
      mimeType: AGENT_HARDWARE_LOOP_MIME,
      text: AGENT_HARDWARE_LOOP_TEXT,
    };
  }
  if (HARDWARE_LAB_TEMPLATE_ALIASES.includes(uri as typeof HARDWARE_LAB_TEMPLATE_ALIASES[number])) {
    return {
      uri,
      mimeType: HARDWARE_LAB_TEMPLATE_MIME,
      text: HARDWARE_LAB_TEMPLATE_TEXT,
      _meta: hardwareLabComponentMeta(),
    };
  }
  return null;
}
