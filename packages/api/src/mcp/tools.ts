import type { Env } from '../types.js';
import type { HostedMcpIdentity, McpTool, McpToolResult } from './types.js';
import { diagramToConfig, COMPONENT_META, composeDiagnostics, compile } from '@labwired/board-config';
import type { ValidateDiagram } from '@labwired/board-config';
import { builderRun } from './builder-client.js';
import { getWorkspaceRecord, maybeResetMtdCycles, writeWorkspaceRecord } from '../keys.js';
import { trackUsage } from '../usage.js';
import {
  AGENT_HARDWARE_LOOP_GUIDE_URI,
  HOSTED_AGENT_WORKFLOW,
  SEARCH_TOOLS_TOOL,
  SEARCH_TOOLS_TOOL_NAME,
  rankTools,
} from './search-tools.js';
import { decorateTools } from './tool-metadata.js';
import { HARDWARE_LAB_TEMPLATE_URI } from './resources.js';

const hostedTools: McpTool[] = [
  SEARCH_TOOLS_TOOL,
  {
    name: 'labwired_start_playground_lab',
    description:
      'Zero-friction first run: create a Playground watch session, build a starter virtual hardware lab, validate it, and return a watch URL.',
    inputSchema: {
      type: 'object',
      properties: {
        goal: {
          type: 'string',
          description: 'Optional natural-language goal. Defaults to a simple STM32 LED circuit.',
        },
        board: {
          type: 'string',
          description: 'Optional board id. Defaults to stm32l476-blinky.',
        },
        run: {
          type: 'boolean',
          description: 'Whether to start from a runnable demo lab. Defaults to true.',
        },
      },
    },
  },
  {
    name: 'labwired_open_hardware_lab',
    description:
      'Open an embeddable visual hardware lab for an agent-generated board diagram. ' +
      'Returns a browser watch URL plus structured scene data; ChatGPT-capable clients can render the bundled hardware lab component inline.',
    _meta: {
      'openai/outputTemplate': HARDWARE_LAB_TEMPLATE_URI,
      'openai/widgetAccessible': true,
    },
    inputSchema: {
      type: 'object',
      properties: {
        diagram: {
          type: 'object',
          description: 'Optional diagram JSON with board, parts, and wires. Defaults to a starter STM32 LED lab.',
        },
        title: {
          type: 'string',
          description: 'Optional display title for the visual lab.',
        },
      },
    },
    outputSchema: {
      type: 'object',
      required: ['ok', 'watch_url', 'template_uri', 'scene'],
      properties: {
        ok: { type: 'boolean' },
        watch_url: { type: 'string' },
        template_uri: { type: 'string' },
        scene: { type: 'object' },
        evidence: { type: 'object' },
      },
    },
  },
  {
    name: 'labwired_list_boards',
    description: 'List hosted Playground starter boards available to agents.',
    inputSchema: {
      type: 'object',
      properties: {
        filter: { type: 'string', description: 'Optional substring filter.' },
      },
    },
  },
  {
    name: 'labwired_validate_diagram',
    description: 'Validate a Playground diagram before running or sharing it.',
    inputSchema: {
      type: 'object',
      required: ['diagram'],
      properties: {
        diagram: {
          type: 'object',
          description: 'Diagram JSON with board, parts, and wires.',
        },
      },
    },
  },
  {
    name: 'labwired_run',
    description:
      'Run a compiled ELF firmware in the LabWired digital-twin simulator against a virtual hardware diagram.' +
      ' Returns run status, serial output, cycle counts, and — on fault, hang, or step-limit — a hardware-level' +
      ' diagnosis explaining what went wrong and why (e.g. infinite loop, unmodeled peripheral poll, bad pointer).' +
      ' The caller must supply a compiled ELF; see docs/firmware-scaffolds/README.md for the exact arm-none-eabi-gcc' +
      ' flags and linker script to produce a bootable ELF for stm32l476.',
    inputSchema: {
      type: 'object',
      required: ['elf_base64', 'target', 'diagram'],
      properties: {
        elf_base64: { type: 'string', description: 'Base64-encoded ELF firmware binary.' },
        target: { type: 'string', enum: ['stm32l476'], description: 'Target MCU identifier, must match diagram.board.' },
        diagram: { type: 'object', description: 'Diagram JSON with board, parts, and wires.' },
        max_steps: { type: 'number', description: 'Maximum simulation steps (default 1,000,000).' },
      },
    },
  },
  {
    name: 'labwired_list_components',
    description: 'List all available virtual hardware components and their board_io kinds.',
    inputSchema: {
      type: 'object',
      properties: {},
    },
  },
  {
    name: 'labwired_compile_diagram',
    description:
      'Compile a wired diagram into a LabWired System Manifest YAML. ' +
      'Runs ERC first — errors abort compilation. Returns system_yaml inline ' +
      '(no filesystem persistence in hosted mode). Use after labwired_validate_diagram. ' +
      'Keywords: compile diagram, diagram to manifest, build board.',
    inputSchema: {
      type: 'object',
      required: ['diagram'],
      properties: {
        diagram: {
          type: 'object',
          description: 'Diagram JSON with board, parts, and wires.',
        },
        name: {
          type: 'string',
          description: 'Optional name for the output (informational only in hosted mode).',
        },
      },
    },
  },
];

export function listHostedTools(): McpTool[] {
  return decorateTools(hostedTools);
}

export async function callHostedTool(
  params: unknown,
  env: Env,
  identity: HostedMcpIdentity,
): Promise<McpToolResult> {
  const parsed = params as { name?: unknown; arguments?: unknown } | null;
  const name = typeof parsed?.name === 'string' ? parsed.name : '';
  const started = Date.now();
  const result = await dispatchHostedTool(parsed, name, env, identity);
  trackUsage(env, {
    event: 'mcp_tool',
    tool: name,
    board: boardFromArgs(parsed?.arguments),
    status: result.isError ? 'error' : 'ok',
    durationMs: Date.now() - started,
  });
  return result;
}

/** Best-effort board/target extraction for usage stats. */
function boardFromArgs(args: unknown): string | undefined {
  const a = args as { target?: unknown; diagram?: { board?: unknown } } | null;
  if (typeof a?.target === 'string') return a.target;
  if (typeof a?.diagram?.board === 'string') return a.diagram.board;
  return undefined;
}

async function dispatchHostedTool(
  parsed: { name?: unknown; arguments?: unknown } | null,
  name: string,
  env: Env,
  identity: HostedMcpIdentity,
): Promise<McpToolResult> {

  if (name === SEARCH_TOOLS_TOOL_NAME) {
    const input = (parsed?.arguments ?? {}) as { query?: unknown; limit?: unknown };
    const query = typeof input.query === 'string' ? input.query : '';
    const limit = typeof input.limit === 'number' && Number.isFinite(input.limit)
      ? Math.trunc(input.limit)
      : 8;
    return {
      content: [textContent({
        query,
        guide_uri: AGENT_HARDWARE_LOOP_GUIDE_URI,
        workflow: HOSTED_AGENT_WORKFLOW,
        tools: rankTools(query, listHostedTools(), limit),
      })],
    };
  }

  if (name === 'labwired_start_playground_lab') {
    return startPlaygroundLab(parsed?.arguments, env, identity);
  }

  if (name === 'labwired_open_hardware_lab') {
    return openHardwareLab(parsed?.arguments, env, identity);
  }

  if (name === 'labwired_list_boards') {
    return {
      content: [
        textContent({
          boards: [
            {
              id: 'stm32l476-blinky',
              name: 'STM32L476 LED starter',
              description: 'STM32L476 with an LED on PA5; best first hosted lab.',
              board: 'stm32l476',
              target: 'stm32l476',
              languages: ['c', 'cpp'],
            },
          ],
        }),
      ],
    };
  }

  if (name === 'labwired_validate_diagram') {
    const args = (parsed?.arguments ?? {}) as { diagram?: unknown };
    const diagram = args.diagram;
    if (!diagram || typeof diagram !== 'object' || Array.isArray(diagram)) {
      return {
        content: [textContent({ ok: false, error_count: 1, warning_count: 0, diagnostics: [{ severity: 'error', code: 'DIAGRAM_MALFORMED', message: 'Diagram must be an object with board, parts, and wires.' }] })],
        isError: true,
      };
    }
    const validation = composeDiagnostics(diagram as unknown as ValidateDiagram);
    return {
      content: [textContent(validation)],
      isError: validation.error_count > 0 || undefined,
    };
  }

  if (name === 'labwired_compile_diagram') {
    const input = (parsed?.arguments ?? {}) as { diagram?: unknown; name?: unknown };
    const diagram = input.diagram;
    if (!diagram || typeof diagram !== 'object' || Array.isArray(diagram)) {
      return {
        content: [textContent({ error: 'INVALID_ARGS', detail: 'diagram is required and must be an object' })],
        isError: true,
      };
    }
    const result = compile(diagram as Parameters<typeof compile>[0]);
    if (!result.ok) {
      return {
        content: [textContent({ ok: false, diagnostics: result.diagnostics })],
        isError: true,
      };
    }
    return {
      content: [textContent({ ok: true, system_yaml: result.systemYaml, diagnostics: result.diagnostics })],
    };
  }

  if (name === 'labwired_run') {
    return handleRun(parsed?.arguments, env, identity);
  }

  if (name === 'labwired_list_components') {
    const components = Object.entries(COMPONENT_META)
      .filter(([, m]) => m.boardIoKind)
      .map(([type, m]) => ({ type, board_io_kind: m.boardIoKind }));
    return { content: [textContent({ components })] };
  }

  return {
    content: [textContent({ error: 'UNKNOWN_TOOL', name })],
    isError: true,
  };
}

function textContent(value: unknown): { type: 'text'; text: string } {
  return { type: 'text', text: JSON.stringify(value) };
}

async function handleRun(
  args: unknown,
  env: Env,
  identity: HostedMcpIdentity,
): Promise<McpToolResult> {
  const input = (args ?? {}) as Record<string, unknown>;
  const elfBase64 = typeof input.elf_base64 === 'string' && input.elf_base64 ? input.elf_base64 : null;
  const target = typeof input.target === 'string' && input.target ? input.target : null;
  const diagram = input.diagram;
  const maxSteps = typeof input.max_steps === 'number' ? input.max_steps : 1_000_000;

  if (!elfBase64 || !target || !diagram) {
    return {
      content: [textContent({ error: 'INVALID_ARGS', detail: 'elf_base64, target, and diagram are required' })],
      isError: true,
    };
  }

  // Consistency guard: target must match diagram.board
  const diagramBoard = typeof (diagram as Record<string, unknown>).board === 'string'
    ? (diagram as Record<string, unknown>).board as string
    : null;
  if (!diagramBoard || diagramBoard !== target) {
    return {
      content: [textContent({ error: `TARGET_BOARD_MISMATCH: target=${target} but diagram.board=${diagramBoard ?? 'missing'}` })],
      isError: true,
    };
  }

  // Convert diagram to system + chip YAML
  let systemYaml: string;
  let chipYaml: string;
  try {
    const config = diagramToConfig(diagram as Parameters<typeof diagramToConfig>[0]);
    systemYaml = config.systemYaml;
    chipYaml = config.chipYaml;
  } catch (err) {
    return {
      content: [textContent({ error: 'DIAGRAM_INVALID', detail: String(err) })],
      isError: true,
    };
  }

  const result = await builderRun(env, { elfBase64, systemYaml, chipYaml, maxSteps });

  // Meter cycles against workspace if present
  if (identity.workspaceId && result.status !== 'error') {
    await meterRunCycles(env, identity.workspaceId, result.cycles).catch(() => {
      // best-effort; don't fail the run response on metering errors
    });
  }

  return {
    content: [textContent(result)],
    isError: result.status === 'error' ? true : undefined,
  };
}

async function meterRunCycles(env: Env, workspaceId: string, cycles: number): Promise<void> {
  const workspace = await getWorkspaceRecord(env, workspaceId);
  if (!workspace) return;
  const updated = await maybeResetMtdCycles(env, workspaceId, workspace);
  updated.cycles_used_mtd += cycles;
  await writeWorkspaceRecord(env, workspaceId, updated);
}

function randomHex(bytes: number): string {
  const buf = new Uint8Array(bytes);
  crypto.getRandomValues(buf);
  return Array.from(buf, (b) => b.toString(16).padStart(2, '0')).join('');
}

async function startPlaygroundLab(
  args: unknown,
  env: Env,
  identity: HostedMcpIdentity,
): Promise<McpToolResult> {
  const input = (args ?? {}) as { goal?: unknown; board?: unknown; run?: unknown };
  const board = typeof input.board === 'string' && input.board ? input.board : 'stm32l476-blinky';
  const sessionId = `mcp_${randomHex(8)}`;
  const watchUrl = `https://app.labwired.com/?watch=${encodeURIComponent(sessionId)}`;
  const stub = env.SESSIONS.get(env.SESSIONS.idFromName(sessionId));
  const diagram = starterDiagram(board);
  const validation = composeDiagnostics(diagram as unknown as ValidateDiagram);
  if (!validation.ok) {
    return {
      content: [textContent({ error: 'STARTER_DIAGRAM_INVALID', validation })],
      isError: true,
    };
  }

  const initResp = await stub.fetch(new Request('https://session/__init', {
    method: 'POST',
    body: JSON.stringify({
      session_id: sessionId,
      clerk_user_id: identity.userId,
    }),
    headers: { 'Content-Type': 'application/json' },
  }));
  if (!initResp.ok) {
    return {
      content: [textContent({ error: 'SESSION_INIT_FAILED', status: initResp.status })],
      isError: true,
    };
  }

  const init = (await initResp.json().catch(() => null)) as { owner_token?: string } | null;
  if (!init?.owner_token) {
    return {
      content: [textContent({ error: 'SESSION_INIT_FAILED', detail: 'missing owner token' })],
      isError: true,
    };
  }

  const sessionUpdate: Record<string, unknown> = {
    board_id: board,
    diagram,
    owner_user_id: identity.userId,
    status: input.run === false ? 'idle' : 'completed',
    last_sim_state: {
      exit_reason: 'demo_ready',
      final_cycles: 0,
      serial_tail: 'Hosted LabWired connector created a validated starter lab.',
    },
  };
  if (identity.workspaceId) {
    sessionUpdate.workspace_id = identity.workspaceId;
  }

  const stateResp = await stub.fetch(new Request(`https://session/sessions/${sessionId}/state`, {
    method: 'PUT',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${init.owner_token}`,
    },
    body: JSON.stringify(sessionUpdate),
  }));
  if (!stateResp.ok) {
    return {
      content: [textContent({ error: 'SESSION_UPDATE_FAILED', status: stateResp.status })],
      isError: true,
    };
  }

  return {
    content: [
      textContent({
        watch_url: watchUrl,
        summary: 'Created a virtual STM32 LED circuit in the Playground and validated the starter wiring.',
        board_id: board,
        validation,
        next_prompt: 'Ask me to add a button, sensor, UART, or CI check.',
      }),
    ],
  };
}

async function openHardwareLab(
  args: unknown,
  env: Env,
  identity: HostedMcpIdentity,
): Promise<McpToolResult> {
  const input = (args ?? {}) as { diagram?: unknown; title?: unknown };
  const diagram = diagramOrStarter(input.diagram);
  const sessionId = `mcp_${randomHex(8)}`;
  const watchUrl = `https://app.labwired.com/?watch=${encodeURIComponent(sessionId)}`;
  const scene = sceneFromDiagram(diagram);
  const evidence = {
    status: 'ready',
    diagnostics: composeDiagnostics(diagram as unknown as ValidateDiagram).diagnostics,
  };
  const structuredContent = {
    ok: true,
    title: typeof input.title === 'string' && input.title ? input.title : 'LabWired Hardware Lab',
    watch_url: watchUrl,
    template_uri: HARDWARE_LAB_TEMPLATE_URI,
    scene,
    evidence,
  };

  await seedHardwareLabSession(env, identity, sessionId, structuredContent).catch(() => {
    // Embedding still works without the live browser session; the watch URL is best-effort.
  });

  return {
    structuredContent,
    _meta: {
      'openai/outputTemplate': HARDWARE_LAB_TEMPLATE_URI,
      scene,
      evidence,
    },
    content: [
      textContent({
        watch_url: watchUrl,
        template_uri: HARDWARE_LAB_TEMPLATE_URI,
        summary: 'Opened an embeddable LabWired hardware lab for the current diagram.',
      }),
    ],
  };
}

function diagramOrStarter(diagram: unknown): Record<string, unknown> {
  if (diagram && typeof diagram === 'object' && !Array.isArray(diagram)) {
    return diagram as Record<string, unknown>;
  }
  return starterDiagram('stm32l476-blinky');
}

function sceneFromDiagram(diagram: Record<string, unknown>): Record<string, unknown> {
  return {
    board: typeof diagram.board === 'string' ? diagram.board : 'stm32l476',
    parts: Array.isArray(diagram.parts) ? diagram.parts : [],
    wires: Array.isArray(diagram.wires) ? diagram.wires : [],
    nets: Array.isArray(diagram.nets) ? diagram.nets : [],
  };
}

async function seedHardwareLabSession(
  env: Env,
  identity: HostedMcpIdentity,
  sessionId: string,
  structuredContent: Record<string, unknown>,
): Promise<void> {
  const sessions = (env as { SESSIONS?: Env['SESSIONS'] }).SESSIONS;
  if (!sessions) return;

  const stub = sessions.get(sessions.idFromName(sessionId));
  const initResp = await stub.fetch(new Request('https://session/__init', {
    method: 'POST',
    body: JSON.stringify({
      session_id: sessionId,
      clerk_user_id: identity.userId,
    }),
    headers: { 'Content-Type': 'application/json' },
  }));
  if (!initResp.ok) return;

  const init = (await initResp.json().catch(() => null)) as { owner_token?: string } | null;
  if (!init?.owner_token) return;

  await stub.fetch(new Request(`https://session/sessions/${sessionId}/state`, {
    method: 'PUT',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${init.owner_token}`,
    },
    body: JSON.stringify({
      owner_user_id: identity.userId,
      status: 'ready',
      diagram: structuredContent.scene,
      last_sim_state: structuredContent.evidence,
    }),
  }));
}

function boardChipForLabId(labId: string): string {
  if (labId === 'stm32l476-blinky') return 'stm32l476';
  // Fall back to the id itself if it doesn't match a known lab entry.
  return labId;
}

function starterDiagram(labId: string): Record<string, unknown> {
  const chip = boardChipForLabId(labId);
  return {
    board: chip,
    parts: [
      { id: 'mcu', type: 'mcu', label: 'STM32L476' },
      { id: 'led1', type: 'led', label: 'LED', color: 'green' },
    ],
    wires: [
      { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } },
    ],
  };
}
