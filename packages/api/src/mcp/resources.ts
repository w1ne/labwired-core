export const AGENT_HARDWARE_LOOP_URI = 'labwired://guides/agent-hardware-loop';
export const AGENT_HARDWARE_LOOP_NAME = 'labwired-agent-hardware-loop';
export const AGENT_HARDWARE_LOOP_MIME = 'text/markdown';
export const HARDWARE_LAB_TEMPLATE_URI = 'ui://labwired/hardware-lab.html';
export const HARDWARE_LAB_TEMPLATE_NAME = 'labwired-hardware-lab';
export const HARDWARE_LAB_TEMPLATE_MIME = 'text/html;profile=mcp-app';
export const HARDWARE_LAB_WIDGET_DOMAIN = 'https://labwired.com';

export const HARDWARE_LAB_WIDGET_CSP = {
  connect_domains: ['https://app.labwired.com', 'https://api.labwired.com'],
  resource_domains: ['https://labwired.com', 'https://app.labwired.com'],
};

const AGENT_HARDWARE_LOOP_TEXT = `# LabWired agent hardware loop

Use LabWired as a deterministic virtual hardware lab for firmware work.

1. Choose a board with \`labwired_list_boards\`.
2. Discover modeled peripherals with \`labwired_list_components\`.
3. Build or update the diagram with an MCU, components, and wires.
4. Validate the diagram with \`labwired_validate_diagram\` before running.
5. Compile firmware outside hosted MCP using the documented scaffold and target flags.
6. Run the ELF with \`labwired_run\`, passing \`elf_base64\`, \`target\`, and \`diagram\`. The hosted connector compiles the diagram internally — pass the diagram object directly, not a system YAML. Use \`labwired_compile_diagram\` separately if you need to inspect or download the compiled \`system_yaml\`.
7. Inspect serial output, cycle counts, stop reasons, and hardware diagnosis.
8. Iterate on firmware or wiring until simulator evidence matches the intended behavior.

Hosted MCP accepts Clerk OAuth bearer tokens and \`lwk_live_\` workspace API keys. The hosted connector runs firmware through the LabWired builder; it does not compile source.
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
      main { display: grid; grid-template-rows: auto minmax(420px, 1fr) auto; gap: 12px; min-height: 100vh; padding: 14px; box-sizing: border-box; }
      header { display: flex; justify-content: space-between; gap: 12px; align-items: center; }
      h1 { font-size: 18px; margin: 0; font-weight: 650; }
      a { color: #67e8f9; }
      .frame-shell { min-height: 420px; border: 1px solid #334155; border-radius: 8px; background: #020617; overflow: hidden; }
      iframe { display: block; width: 100%; height: 100%; min-height: 420px; border: 0; background: #020617; }
      pre { margin: 0; overflow: auto; background: #020617; border: 1px solid #334155; border-radius: 8px; padding: 12px; max-height: 160px; }
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
        <a id="watch" target="_blank" rel="noreferrer" aria-disabled="true">Waiting for lab link</a>
      </header>
      <section class="frame-shell" aria-label="LabWired embedded hardware lab">
        <iframe
          id="labwired-frame"
          title="LabWired Studio live device"
          sandbox="allow-scripts allow-same-origin"
          referrerpolicy="no-referrer"
        ></iframe>
      </section>
      <pre id="json"></pre>
    </main>
    <script>
      const frame = document.getElementById('labwired-frame');
      const json = document.getElementById('json');
      const watch = document.getElementById('watch');
      function toolData(value) {
        if (!value || typeof value !== 'object') return {};
        if (value.structuredContent && typeof value.structuredContent === 'object') return value.structuredContent;
        if (value.toolOutput && typeof value.toolOutput === 'object') return value.toolOutput;
        return value;
      }
      function render(value) {
        const data = toolData(value);
        const scene = data.scene ?? {};
        const frameUrl = data.inline_frame_url ?? data.studio_url ?? data.share_url ?? '';
        if (frameUrl) {
          if (frame.src !== frameUrl) frame.src = frameUrl;
          watch.href = data.studio_url ?? frameUrl;
          watch.textContent = 'Open in LabWired Studio';
          watch.setAttribute('aria-disabled', 'false');
        } else {
          frame.removeAttribute('src');
          watch.removeAttribute('href');
          watch.textContent = 'Waiting for lab link';
          watch.setAttribute('aria-disabled', 'true');
        }
        json.textContent = JSON.stringify({ inline_frame_url: frameUrl, board: scene.board, parts: scene.parts ?? [], wires: scene.wires ?? [], evidence: data.evidence ?? {} }, null, 2);
      }
      render(window.openai?.toolOutput ?? window.openai?.structuredContent);
      window.addEventListener('message', (event) => {
        const message = event.data;
        if (!message || typeof message !== 'object') return;
        if (message.method === 'ui/notifications/tool-result') {
          render(message.params);
        }
      });
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
    ui: {
      domain: HARDWARE_LAB_WIDGET_DOMAIN,
      csp: HARDWARE_LAB_WIDGET_CSP,
    },
    'openai/widgetDescription': 'Interactive LabWired hardware board, firmware state, and simulator evidence.',
    'openai/widgetCSP': HARDWARE_LAB_WIDGET_CSP,
    'openai/widgetDomain': HARDWARE_LAB_WIDGET_DOMAIN,
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
  if (uri === HARDWARE_LAB_TEMPLATE_URI) {
    return {
      uri,
      mimeType: HARDWARE_LAB_TEMPLATE_MIME,
      text: HARDWARE_LAB_TEMPLATE_TEXT,
      _meta: hardwareLabComponentMeta(),
    };
  }
  return null;
}
