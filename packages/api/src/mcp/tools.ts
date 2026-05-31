import type { Env } from '../types.js';
import type { HostedMcpIdentity, McpTool, McpToolResult } from './types.js';

type DiagnosticSeverity = 'error' | 'warning';

interface HostedDiagnostic {
  severity: DiagnosticSeverity;
  code:
    | 'DIAGRAM_MALFORMED'
    | 'UNKNOWN_COMPONENT'
    | 'WIRE_INVALID_PART'
    | 'WIRE_SELF_LOOP'
    | 'PIN_NOT_ON_CHIP'
    | 'BOARDIO_MULTIPLE_WIRES'
    | 'NO_MCU'
    | 'COMPONENT_DANGLING';
  message: string;
  location?: { part_id?: string; pin?: string };
  fix?: string;
}

interface HostedValidationResult {
  ok: boolean;
  error_count: number;
  warning_count: number;
  diagnostics: HostedDiagnostic[];
}

interface DiagramPart {
  id: string;
  type: string;
}

interface WireEndpoint {
  part: string;
  pin: string;
}

interface DiagramWire {
  from: WireEndpoint;
  to: WireEndpoint;
}

interface HostedDiagram {
  board: string;
  parts: DiagramPart[];
  wires: DiagramWire[];
}

const hostedTools: McpTool[] = [
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
          description: 'Optional board id. Defaults to stm32f103-blinky.',
        },
        run: {
          type: 'boolean',
          description: 'Whether to start from a runnable demo lab. Defaults to true.',
        },
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
];

export function listHostedTools(): McpTool[] {
  return hostedTools;
}

export async function callHostedTool(
  params: unknown,
  env: Env,
  identity: HostedMcpIdentity,
): Promise<McpToolResult> {
  const parsed = params as { name?: unknown; arguments?: unknown } | null;
  const name = typeof parsed?.name === 'string' ? parsed.name : '';

  if (name === 'labwired_start_playground_lab') {
    return startPlaygroundLab(parsed?.arguments, env, identity);
  }

  if (name === 'labwired_list_boards') {
    return {
      content: [
        textContent({
          boards: [
            {
              id: 'stm32f103-blinky',
              name: 'STM32F103 LED starter',
              description: 'STM32F103 with LED on PA5; best first hosted lab.',
            },
          ],
        }),
      ],
    };
  }

  if (name === 'labwired_validate_diagram') {
    const validation = validateHostedDiagram(parsed?.arguments);
    return {
      content: [textContent(validation)],
      isError: validation.error_count > 0 || undefined,
    };
  }

  return {
    content: [textContent({ error: 'UNKNOWN_TOOL', name })],
    isError: true,
  };
}

function textContent(value: unknown): { type: 'text'; text: string } {
  return { type: 'text', text: JSON.stringify(value) };
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
  const board = typeof input.board === 'string' && input.board ? input.board : 'stm32f103-blinky';
  const sessionId = `mcp_${randomHex(8)}`;
  const watchUrl = `https://app.labwired.com/?watch=${encodeURIComponent(sessionId)}`;
  const stub = env.SESSIONS.get(env.SESSIONS.idFromName(sessionId));
  const diagram = starterDiagram(board);
  const validation = validateDiagramValue(diagram);
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

function starterDiagram(board: string): Record<string, unknown> {
  return {
    board,
    parts: [
      { id: 'mcu', type: 'mcu', label: 'STM32F103' },
      { id: 'led1', type: 'led', label: 'LED', color: 'green' },
    ],
    wires: [
      { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' } },
    ],
  };
}

function validateHostedDiagram(args: unknown): HostedValidationResult {
  const input = (args ?? {}) as { diagram?: unknown };
  return validateDiagramValue(input.diagram);
}

function validateDiagramValue(value: unknown): HostedValidationResult {
  const parsed = parseHostedDiagram(value);
  if ('result' in parsed) return parsed.result;
  return summarizeDiagnostics(diagnoseHostedDiagram(parsed.diagram));
}

function parseHostedDiagram(value: unknown): { diagram: HostedDiagram } | { result: HostedValidationResult } {
  const diagnostics: HostedDiagnostic[] = [];
  if (!isRecord(value)) {
    return {
      result: summarizeDiagnostics([
        {
          severity: 'error',
          code: 'DIAGRAM_MALFORMED',
          message: 'Diagram must be an object with board, parts, and wires.',
        },
      ]),
    };
  }

  if (typeof value.board !== 'string' || value.board.trim() === '') {
    diagnostics.push({
      severity: 'error',
      code: 'DIAGRAM_MALFORMED',
      message: 'Diagram board must be a non-empty string.',
    });
  }
  if (!Array.isArray(value.parts)) {
    diagnostics.push({
      severity: 'error',
      code: 'DIAGRAM_MALFORMED',
      message: 'Diagram parts must be an array.',
    });
  }
  if (!Array.isArray(value.wires)) {
    diagnostics.push({
      severity: 'error',
      code: 'DIAGRAM_MALFORMED',
      message: 'Diagram wires must be an array.',
    });
  }
  if (diagnostics.length > 0) {
    return { result: summarizeDiagnostics(diagnostics) };
  }

  const parts = parseParts(value.parts);
  const wires = parseWires(value.wires);
  diagnostics.push(...parts.diagnostics, ...wires.diagnostics);
  if (diagnostics.length > 0) {
    return { result: summarizeDiagnostics(diagnostics) };
  }

  return {
    diagram: {
      // Validated as a non-empty string above (DIAGRAM_MALFORMED guard).
      board: (value.board as string).trim(),
      parts: parts.parts,
      wires: wires.wires,
    },
  };
}

function parseParts(value: unknown): { parts: DiagramPart[]; diagnostics: HostedDiagnostic[] } {
  const parts: DiagramPart[] = [];
  const diagnostics: HostedDiagnostic[] = [];
  for (const [index, part] of (value as unknown[]).entries()) {
    if (!isRecord(part) || typeof part.id !== 'string' || typeof part.type !== 'string') {
      diagnostics.push({
        severity: 'error',
        code: 'DIAGRAM_MALFORMED',
        message: `Diagram part at index ${index} must include string id and type.`,
      });
      continue;
    }
    parts.push({ id: part.id, type: part.type });
  }
  return { parts, diagnostics };
}

function parseWires(value: unknown): { wires: DiagramWire[]; diagnostics: HostedDiagnostic[] } {
  const wires: DiagramWire[] = [];
  const diagnostics: HostedDiagnostic[] = [];
  for (const [index, wire] of (value as unknown[]).entries()) {
    if (!isRecord(wire) || !isEndpoint(wire.from) || !isEndpoint(wire.to)) {
      diagnostics.push({
        severity: 'error',
        code: 'DIAGRAM_MALFORMED',
        message: `Diagram wire at index ${index} must include from/to endpoints with string part and pin.`,
      });
      continue;
    }
    wires.push({ from: wire.from, to: wire.to });
  }
  return { wires, diagnostics };
}

function diagnoseHostedDiagram(diagram: HostedDiagram): HostedDiagnostic[] {
  const diagnostics: HostedDiagnostic[] = [];
  const partsById = new Map(diagram.parts.map((part) => [part.id, part]));
  const componentMcuWireCount = new Map<string, number>();
  const mcuPins = pinsForBoard(diagram.board);

  for (const part of diagram.parts) {
    if (!componentKind(part)) {
      diagnostics.push({
        severity: 'error',
        code: 'UNKNOWN_COMPONENT',
        message: `Component type "${part.type}" is not available in the hosted validator.`,
        location: { part_id: part.id },
      });
    }
  }

  for (const wire of diagram.wires) {
    const fromPart = partsById.get(wire.from.part);
    const toPart = partsById.get(wire.to.part);
    if (!fromPart || !toPart) {
      diagnostics.push({
        severity: 'error',
        code: 'WIRE_INVALID_PART',
        message: `Wire endpoint references unknown part: ${!fromPart ? wire.from.part : wire.to.part}.`,
      });
      continue;
    }
    if (fromPart.id === toPart.id) {
      diagnostics.push({
        severity: 'error',
        code: 'WIRE_SELF_LOOP',
        message: 'A component cannot be wired to itself.',
        location: { part_id: fromPart.id },
      });
      continue;
    }

    const mcuEndpoint = isMcuPart(fromPart) ? wire.from : isMcuPart(toPart) ? wire.to : null;
    const componentEndpoint = mcuEndpoint === wire.from ? wire.to : mcuEndpoint === wire.to ? wire.from : null;
    if (!mcuEndpoint || !componentEndpoint) continue;

    const component = partsById.get(componentEndpoint.part);
    const kind = component ? componentKind(component) : null;
    if (!kind || kind === 'mcu') continue;

    componentMcuWireCount.set(componentEndpoint.part, (componentMcuWireCount.get(componentEndpoint.part) ?? 0) + 1);
    if (!isPowerPin(mcuEndpoint.pin) && !mcuPins.has(mcuEndpoint.pin.toUpperCase())) {
      diagnostics.push({
        severity: 'error',
        code: 'PIN_NOT_ON_CHIP',
        message: `Pin ${mcuEndpoint.pin} is not available on this board model.`,
        location: { part_id: componentEndpoint.part, pin: mcuEndpoint.pin },
        fix: 'Pick a pin that exists on the selected board.',
      });
    }
  }

  for (const [partId, count] of componentMcuWireCount) {
    const part = partsById.get(partId);
    const kind = part ? componentKind(part) : null;
    if (count > 1 && (kind === 'led' || kind === 'button')) {
      diagnostics.push({
        severity: 'error',
        code: 'BOARDIO_MULTIPLE_WIRES',
        message: `${part?.type ?? partId} has ${count} MCU connections; expected exactly one hosted board_io wire.`,
        location: { part_id: partId },
      });
    }
  }

  if (!diagram.parts.some(isMcuPart)) {
    diagnostics.push({
      severity: 'error',
      code: 'NO_MCU',
      message: 'Diagram has no MCU. Add a board before simulating.',
    });
  }

  for (const part of diagram.parts) {
    const kind = componentKind(part);
    if ((kind === 'led' || kind === 'button') && (componentMcuWireCount.get(part.id) ?? 0) === 0) {
      diagnostics.push({
        severity: 'warning',
        code: 'COMPONENT_DANGLING',
        message: `${part.type} has no MCU connection; it will not be simulated.`,
        location: { part_id: part.id },
      });
    }
  }

  return diagnostics;
}

function summarizeDiagnostics(diagnostics: HostedDiagnostic[]): HostedValidationResult {
  const errorCount = diagnostics.filter((diag) => diag.severity === 'error').length;
  const warningCount = diagnostics.filter((diag) => diag.severity === 'warning').length;
  return {
    ok: errorCount === 0,
    error_count: errorCount,
    warning_count: warningCount,
    diagnostics,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isEndpoint(value: unknown): value is WireEndpoint {
  return isRecord(value) && typeof value.part === 'string' && typeof value.pin === 'string';
}

function componentKind(part: DiagramPart): 'mcu' | 'led' | 'button' | null {
  if (part.id === 'mcu' || part.type === 'mcu' || part.type === 'stm32-dev') return 'mcu';
  if (part.type === 'led' || part.type === 'rgb-led') return 'led';
  if (part.type === 'button' || part.type === 'slide-switch') return 'button';
  return null;
}

function isMcuPart(part: DiagramPart): boolean {
  return componentKind(part) === 'mcu';
}

function pinsForBoard(board: string): Set<string> {
  const normalized = board.startsWith('stm32f103') ? 'stm32f103' : board;
  if (normalized !== 'stm32f103') return new Set();
  return new Set([
    'PA0', 'PA1', 'PA2', 'PA3', 'PA4', 'PA5', 'PA6', 'PA7', 'PA8', 'PA9', 'PA10', 'PA11',
    'PA12', 'PA13', 'PA14', 'PA15', 'PB0', 'PB1', 'PB3', 'PB4', 'PB5', 'PB6', 'PB7', 'PB8',
    'PB9', 'PB10', 'PB11', 'PB12', 'PB13', 'PB14', 'PB15', 'PC0', 'PC1', 'PC2', 'PC3',
    'PC4', 'PC5', 'PC6', 'PC7', 'PC8', 'PC9', 'PC10', 'PC11', 'PC12', 'PC13', 'PC14',
    'PC15',
  ]);
}

function isPowerPin(pin: string): boolean {
  return ['VCC', 'GND', '3V3', '5V', 'VIN', 'VBUS', 'VDD', 'VSS'].includes(pin.toUpperCase());
}
