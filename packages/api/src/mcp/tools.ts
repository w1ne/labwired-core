import type { Env } from '../types.js';
import type { HostedMcpIdentity, McpTool, McpToolResult } from './types.js';
import { diagramToConfig, COMPONENT_META, composeDiagnostics, compile, getPlaygroundBoard, listPlaygroundBoards, normalizeLabWiredDiagramV1 } from '@labwired/board-config';
import type { ValidateDiagram } from '@labwired/board-config';
import { builderRun, builderCompile } from './builder-client.js';
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
import { createShareRecord, shareUrls } from '../shares.js';

// Boards the hosted PlatformIO compiler accepts. MUST mirror PIO_BOARDS in
// services/labwired-builder/src/compile.ts. `runnable: false` = compiles but
// the digital twin cannot execute it yet (ESP32 Xtensa/RISC-V boot is open
// sim work), so labwired_build_and_run will compile then stop short of run.
const HOSTED_COMPILE_BOARDS: { board: string; runnable: boolean }[] = [
  { board: 'stm32l476', runnable: true },
  { board: 'nucleo-f401re', runnable: true },
  { board: 'stm32-blackpill', runnable: true },
  { board: 'nucleo-h563zi', runnable: true },
  { board: 'rpi-pico', runnable: true },
  { board: 'nrf52840-dk', runnable: true },
  { board: 'esp32', runnable: false },
  { board: 'esp32-s3-zero', runnable: false },
  { board: 'esp32-c3-supermini', runnable: false },
];
const COMPILE_BOARD_IDS = HOSTED_COMPILE_BOARDS.map((b) => b.board);
const RUNNABLE_BOARD_IDS = new Set(HOSTED_COMPILE_BOARDS.filter((b) => b.runnable).map((b) => b.board));

function hardwareLabToolMeta(): Record<string, unknown> {
  return {
    'openai/outputTemplate': HARDWARE_LAB_TEMPLATE_URI,
    'openai/widgetAccessible': true,
    ui: {
      resourceUri: HARDWARE_LAB_TEMPLATE_URI,
    },
    widgetAccessible: true,
    invoking: 'Opening hardware lab',
    invoked: 'Hardware lab opened',
  };
}

const hostedTools: McpTool[] = [
  SEARCH_TOOLS_TOOL,
  {
    name: 'labwired_start_playground_lab',
    description:
      'Zero-friction first run: build a starter virtual hardware lab, validate it, and return a LabWired Playground URL.',
    _meta: {
      ...hardwareLabToolMeta(),
    },
    inputSchema: {
      type: 'object',
      properties: {
        goal: {
          type: 'string',
          description: 'Optional natural-language goal. Defaults to a simple STM32 LED circuit.',
        },
        board: {
          type: 'string',
          description: 'Optional Playground catalog board id from labwired_list_boards. Defaults to stm32f103-blinky.',
        },
        run: {
          type: 'boolean',
          description: 'Whether to start from a runnable demo lab. Defaults to true.',
        },
      },
    },
    outputSchema: {
      type: 'object',
      required: ['ok', 'inline_component_uri', 'inline_frame_url', 'studio_url', 'share_url', 'scene'],
      properties: {
        ok: { type: 'boolean' },
        inline_component_uri: { type: 'string' },
        inline_frame_url: { type: 'string' },
        studio_url: { type: 'string' },
        share_url: { type: 'string' },
        scene: { type: 'object' },
        evidence: { type: 'object' },
      },
    },
  },
  {
    name: 'labwired_open_hardware_lab',
    description:
      'Open an embeddable visual hardware lab for an agent-generated board diagram. ' +
      'Returns both an inline component URI for agent-side inspection and a shareable LabWired Studio URL for the full device session.',
    _meta: {
      ...hardwareLabToolMeta(),
    },
    inputSchema: {
      type: 'object',
      properties: {
        diagram: {
          type: 'object',
          description: 'Optional diagram JSON with board, parts, and wires. Defaults to a starter STM32 LED lab.',
        },
        source_code: {
          type: 'string',
          description: 'Optional firmware/source code to open in the LabWired Playground editor with this hardware.',
        },
        title: {
          type: 'string',
          description: 'Optional display title for the visual lab.',
        },
      },
    },
    outputSchema: {
      type: 'object',
      required: ['ok', 'inline_component_uri', 'inline_frame_url', 'studio_url', 'share_url', 'scene'],
      properties: {
        ok: { type: 'boolean' },
        inline_component_uri: { type: 'string' },
        inline_frame_url: { type: 'string' },
        studio_url: { type: 'string' },
        share_url: { type: 'string' },
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
  {
    name: 'labwired_compile_firmware',
    description:
      'Compile C/C++ firmware source into an ELF on LabWired\'s hosted PlatformIO toolchain — no local compiler needed. ' +
      'Returns a base64 ELF you can pass straight to labwired_run, or structured compiler diagnostics (file/line/message) on failure. ' +
      'Cortex-M targets (STM32, RP2040, nRF52) both compile and run in the digital twin; ESP32 targets compile but cannot be simulated yet. ' +
      'For one-shot build+simulate, use labwired_build_and_run.',
    inputSchema: {
      type: 'object',
      required: ['source', 'board'],
      properties: {
        source: { type: 'string', description: 'Firmware source (single translation unit). Compiled against a fixed board scaffold.' },
        board: { type: 'string', enum: COMPILE_BOARD_IDS, description: 'Target board id from labwired_list_boards.' },
        language: { type: 'string', enum: ['c', 'cpp'], description: 'Source language. Defaults to c.' },
      },
    },
    outputSchema: {
      type: 'object',
      required: ['ok'],
      properties: {
        ok: { type: 'boolean' },
        elf_base64: { type: 'string' },
        runnable: { type: 'boolean', description: 'Whether the digital twin can execute this target today.' },
        diagnostics: { type: 'array', items: { type: 'object' } },
      },
    },
  },
  {
    name: 'labwired_build_and_run',
    description:
      'One call: compile C/C++ firmware on the hosted toolchain AND run the resulting ELF in the LabWired digital twin against a diagram. ' +
      'The closed write→build→run→diagnose loop for agents with no local toolchain. Aborts with compiler diagnostics if the build fails, ' +
      'or with DIAGRAM_INVALID if the wiring is invalid. Only Cortex-M targets currently execute; ESP32 will compile but not run.',
    inputSchema: {
      type: 'object',
      required: ['source', 'board', 'diagram'],
      properties: {
        source: { type: 'string', description: 'Firmware source.' },
        board: { type: 'string', enum: COMPILE_BOARD_IDS, description: 'Target board id; must equal diagram.board.' },
        diagram: { type: 'object', description: 'Diagram JSON with board, parts, and wires.' },
        language: { type: 'string', enum: ['c', 'cpp'], description: 'Source language. Defaults to c.' },
        max_steps: { type: 'number', description: 'Maximum simulation steps (default 1,000,000).' },
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
    const input = (parsed?.arguments ?? {}) as { filter?: unknown };
    const filter = typeof input.filter === 'string' ? input.filter : undefined;
    return {
      content: [
        textContent({
          boards: listPlaygroundBoards(filter),
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
    const invalid = diagramValidationGate(diagram);
    if (invalid) return invalid;
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

  if (name === 'labwired_compile_firmware') {
    return handleCompileFirmware(parsed?.arguments, env);
  }

  if (name === 'labwired_build_and_run') {
    return handleBuildAndRun(parsed?.arguments, env, identity);
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

/**
 * First-class validation gate. EVERY tool that ingests an agent-authored
 * diagram and then builds, compiles, runs, or publishes hardware MUST call
 * this first: a diagram with ERC/pin errors (unknown pin, no MCU, undriven
 * rail, dangling functional part, …) must never reach the compiler, the
 * simulator, or a shared link. Returns a DIAGRAM_INVALID result to
 * short-circuit the tool, or null when the diagram is clean. Warnings do not
 * block (composeDiagnostics().ok is error-count based).
 */
function diagramValidationGate(diagram: unknown): McpToolResult | null {
  const validation = composeDiagnostics(diagram as unknown as ValidateDiagram);
  if (validation.ok) return null;
  return {
    content: [textContent({
      error: 'DIAGRAM_INVALID',
      detail: 'The diagram has wiring errors and was rejected before building. Fix the errors below, then retry. Call labwired_validate_diagram to re-check.',
      validation,
    })],
    isError: true,
  };
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

  // First-class validation: a board with ERC/pin errors must never reach the
  // simulator, even with a valid ELF and matching target. Block before we
  // lower the diagram to YAML (diagramToConfig silently drops unresolvable
  // pins, so it would otherwise "run" a board the user never actually wired).
  const invalid = diagramValidationGate(diagram);
  if (invalid) return invalid;

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

function compileArgs(args: unknown): { source: string; board: string; language?: 'c' | 'cpp' } | McpToolResult {
  const input = (args ?? {}) as Record<string, unknown>;
  const source = typeof input.source === 'string' ? input.source : '';
  const board = typeof input.board === 'string' ? input.board : '';
  if (!source.trim() || !board) {
    return {
      content: [textContent({ error: 'INVALID_ARGS', detail: 'source and board are required.' })],
      isError: true,
    };
  }
  if (!COMPILE_BOARD_IDS.includes(board)) {
    return {
      content: [textContent({ error: 'BOARD_NOT_COMPILABLE', detail: `Board "${board}" is not supported by the hosted compiler. Supported: ${COMPILE_BOARD_IDS.join(', ')}.` })],
      isError: true,
    };
  }
  const language = input.language === 'cpp' ? 'cpp' : input.language === 'c' ? 'c' : undefined;
  return { source, board, language };
}

function compilerUnavailable(err: unknown): McpToolResult {
  return {
    content: [textContent({
      error: 'COMPILER_UNAVAILABLE',
      detail: 'The hosted firmware compiler is not reachable right now. Compile the ELF in your own sandbox (see docs/firmware-scaffolds) and use labwired_run instead.',
      cause: err instanceof Error ? err.message : String(err),
    })],
    isError: true,
  };
}

async function handleCompileFirmware(args: unknown, env: Env): Promise<McpToolResult> {
  const parsed = compileArgs(args);
  if ('content' in parsed) return parsed;
  let result;
  try {
    result = await builderCompile(env, parsed);
  } catch (err) {
    return compilerUnavailable(err);
  }
  return {
    content: [textContent({
      ok: result.ok,
      ...(result.ok ? { elf_base64: result.elfBase64 } : {}),
      runnable: result.runnable ?? RUNNABLE_BOARD_IDS.has(parsed.board),
      diagnostics: result.diagnostics,
      ...(result.ok ? {} : { detail: 'Firmware did not compile. Fix the diagnostics below and retry.' }),
    })],
    isError: result.ok ? undefined : true,
  };
}

async function handleBuildAndRun(args: unknown, env: Env, identity: HostedMcpIdentity): Promise<McpToolResult> {
  const parsed = compileArgs(args);
  if ('content' in parsed) return parsed;
  const input = (args ?? {}) as Record<string, unknown>;
  const diagram = input.diagram;
  if (!diagram || typeof diagram !== 'object' || Array.isArray(diagram)) {
    return {
      content: [textContent({ error: 'INVALID_ARGS', detail: 'diagram is required and must be an object.' })],
      isError: true,
    };
  }
  // Fail fast if the twin can't execute this board at all.
  if (!RUNNABLE_BOARD_IDS.has(parsed.board)) {
    return {
      content: [textContent({ error: 'BOARD_NOT_RUNNABLE', detail: `Board "${parsed.board}" can be compiled (labwired_compile_firmware) but the digital twin cannot run it yet. Runnable: ${[...RUNNABLE_BOARD_IDS].join(', ')}.` })],
      isError: true,
    };
  }
  // Validate wiring before spending a compile.
  const invalid = diagramValidationGate(diagram);
  if (invalid) return invalid;

  let compiled;
  try {
    compiled = await builderCompile(env, parsed);
  } catch (err) {
    return compilerUnavailable(err);
  }
  if (!compiled.ok) {
    return {
      content: [textContent({ ok: false, stage: 'compile', detail: 'Firmware did not compile. Fix the diagnostics below and retry.', diagnostics: compiled.diagnostics })],
      isError: true,
    };
  }

  // Hand the fresh ELF to the existing run path (validation + metering + diagnosis).
  return handleRun(
    { elf_base64: compiled.elfBase64, target: parsed.board, diagram, max_steps: input.max_steps },
    env,
    identity,
  );
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
  _identity: HostedMcpIdentity,
): Promise<McpToolResult> {
  const input = (args ?? {}) as { goal?: unknown; board?: unknown };
  const board = typeof input.board === 'string' && input.board ? input.board : 'stm32f103-blinky';
  if (!getPlaygroundBoard(board)) {
    return {
      content: [textContent({
        error: 'BOARD_NOT_IN_PLAYGROUND_CATALOG',
        detail: `Unknown Playground board id "${board}". Call labwired_list_boards and use one of its returned id values.`,
      })],
      isError: true,
    };
  }
  const diagram = starterDiagram(board);
  const validation = composeDiagnostics(diagram as unknown as ValidateDiagram);
  if (!validation.ok) {
    return {
      content: [textContent({ error: 'STARTER_DIAGRAM_INVALID', validation })],
      isError: true,
    };
  }
  const source = starterSource();
  const urls = await playgroundUrls(env, diagram, source);
  const scene = sceneFromDiagram(diagram);
  const evidence = {
    status: 'ready',
    diagnostics: validation.diagnostics,
  };
  const structuredContent = {
    ok: true,
    title: 'LabWired Starter Lab',
    inline_component_uri: HARDWARE_LAB_TEMPLATE_URI,
    inline_frame_url: urls.embedUrl,
    studio_url: urls.studioUrl,
    share_url: urls.studioUrl,
    scene,
    evidence,
  };

  return {
    structuredContent,
    _meta: {
      ...hardwareLabToolMeta(),
      scene,
      evidence,
    },
    content: [
      textContent({
        studio_url: urls.studioUrl,
        share_url: urls.studioUrl,
        inline_component_uri: HARDWARE_LAB_TEMPLATE_URI,
        inline_frame_url: urls.embedUrl,
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
  _identity: HostedMcpIdentity,
): Promise<McpToolResult> {
  const input = (args ?? {}) as { diagram?: unknown; title?: unknown; source?: unknown; source_code?: unknown; firmware_source?: unknown };
  const diagram = diagramOrStarter(input.diagram);
  const boardValidationError = validatePlaygroundDiagramBoard(diagram);
  if (boardValidationError) return boardValidationError;
  // Gate: never publish a lab that fails validation (branch's first-class gate).
  // Errors (unknown pin, no MCU, dangling functional part, …) must BLOCK the
  // share — otherwise the agent ships a board that "looks wired but isn't" and it
  // only surfaces when a human opens the link. Return the diagnostics to fix+retry.
  const validation = composeDiagnostics(diagram as unknown as ValidateDiagram);
  if (!validation.ok) {
    return {
      content: [textContent({
        error: 'DIAGRAM_INVALID',
        detail: 'The diagram has wiring errors and was not published. Fix the errors below and call labwired_open_hardware_lab again.',
        validation,
      })],
      isError: true,
    };
  }
  // One shareable link format for everything. The Playground runs the share's
  // own binary if it carries one, else a curated example's firmware matched to
  // the board (prod has no compiler), so the shared lab runs either way.
  const urls = await playgroundUrls(env, diagram, '');
  const embedUrl = urls.embedUrl.replace('?embed=true', '?embed=true&run=1');
  const scene = sceneFromDiagram(diagram);
  const evidence = {
    status: 'ready',
    diagnostics: validation.diagnostics,
  };
  const structuredContent = {
    ok: true,
    title: typeof input.title === 'string' && input.title ? input.title : 'LabWired Hardware Lab',
    inline_component_uri: HARDWARE_LAB_TEMPLATE_URI,
    inline_frame_url: embedUrl,
    studio_url: urls.studioUrl,
    share_url: urls.studioUrl,
    scene,
    evidence,
  };

  return {
    structuredContent,
    _meta: {
      ...hardwareLabToolMeta(),
      scene,
      evidence,
    },
    content: [
      textContent({
        studio_url: urls.studioUrl,
        share_url: urls.studioUrl,
        inline_component_uri: HARDWARE_LAB_TEMPLATE_URI,
        inline_frame_url: embedUrl,
        summary: 'Opened an inline LabWired Playground viewer and a shareable LabWired Studio link for the device.',
      }),
    ],
  };
}

// Curated example labs that ship a pre-built binary (demoFirmwarePath in the
// Playground). Agent diagrams map to the nearest one by their distinctive part,
// so the opened lab is always preloaded and runnable. Order = most specific first.
const EXAMPLE_LAB_BY_PART: Array<{ part: string; id: string }> = [
  { part: 'ultrasonic', id: 'nrf52840-proximity-lab' },
  { part: 'bme280', id: 'bme280-weather-lab' },
  { part: 'mpu6050', id: 'mpu6050-sensor-lab' },
  { part: 'adxl345', id: 'adxl345-sensor-lab' },
  { part: 'max31855', id: 'max31855-thermocouple-lab' },
  { part: 'ntc-thermistor', id: 'ntc-thermistor-lab' },
  { part: 'neo6m-gps', id: 'neo6m-gps-lab' },
  { part: 'oled-ssd1306', id: 'ssd1306-hello-lab' },
  { part: 'ili9341', id: 'ili9341-tft-lab' },
  { part: 'ssd1680_tricolor_290', id: 'epaper-tricolor-lab' },
  { part: 'led-matrix', id: 'nokia5110-invaders-lab' },
  { part: 'iolink-master', id: 'al2205-iolink-dido' },
];

/** Pick the nearest curated example lab (preloaded binary) for a diagram. */
function pickExampleLab(diagram: Record<string, unknown>): string {
  const parts = (Array.isArray(diagram.parts) ? diagram.parts : [])
    .map((p) => (p && typeof p === 'object' ? String((p as Record<string, unknown>).type ?? '') : ''))
    .filter(Boolean);
  for (const { part, id } of EXAMPLE_LAB_BY_PART) {
    if (parts.includes(part)) return id;
  }
  const board = (typeof diagram.board === 'string' ? diagram.board : '').toLowerCase();
  if (board.includes('nrf52840')) return 'nrf52840-proximity-lab';
  if (board.includes('l476') || board.startsWith('stm32l4')) return 'nucleo-l476rg';
  if (board.includes('f401') || board.startsWith('stm32f4')) return 'nucleo-f401re';
  // Default: the classic LED blink, which ships demo-blinky.elf.
  return 'stm32f103-blinky';
}

function exampleLabUrls(boardId: string): { studioUrl: string; embedUrl: string } {
  const id = encodeURIComponent(boardId);
  return {
    studioUrl: `https://app.labwired.com/?board=${id}`,
    embedUrl: `https://app.labwired.com/?embed=true&run=1&board=${id}`,
  };
}

function validatePlaygroundDiagramBoard(diagram: Record<string, unknown>): McpToolResult | null {
  const board = typeof diagram.board === 'string' ? diagram.board : '';
  const allowed = listPlaygroundBoards().some((entry) => entry.id === board || entry.board === board || entry.target === board);
  if (allowed) return null;
  return {
    content: [textContent({
      error: 'BOARD_NOT_IN_PLAYGROUND_CATALOG',
      detail: `diagram.board="${board || 'missing'}" is not in the Playground catalog contract. Call labwired_list_boards and use a returned board/target value.`,
    })],
    isError: true,
  };
}

async function playgroundUrls(env: Env, diagram: Record<string, unknown>, source: string): Promise<{ studioUrl: string; embedUrl: string }> {
  if (env.KV_PROJECTS) {
    const share = await createShareRecord(env, { diagram, source });
    return shareUrls(share.id);
  }
  const encoded = `r${btoa(JSON.stringify({ d: diagram, s: source }))}`;
  return {
    studioUrl: `https://app.labwired.com/#${encoded}`,
    embedUrl: `https://app.labwired.com/?embed=true#${encoded}`,
  };
}

// STM32F1 (e.g. stm32f103 "Bluepill"): PA5 push-pull output via GPIOA_CRL,
// toggled in a busy-wait loop. Registers per the F1 reference manual.
const STM32F1_BLINK_SOURCE = `#include <stdint.h>

#define RCC_APB2ENR (*(volatile uint32_t *)0x40021018u)
#define GPIOA_CRL   (*(volatile uint32_t *)0x40010800u)
#define GPIOA_ODR   (*(volatile uint32_t *)0x4001080Cu)

int main(void) {
  RCC_APB2ENR |= (1u << 2u);
  GPIOA_CRL = (GPIOA_CRL & ~(0xFu << 20u)) | (0x2u << 20u);
  while (1) {
    GPIOA_ODR ^= (1u << 5u);
    for (volatile uint32_t i = 0; i < 100000u; i++) {}
  }
}
`;

// STM32L4 (e.g. stm32l476 Nucleo): enable GPIOA on AHB2, set PA5 to output via
// MODER, toggle ODR. Addresses match the simulator's modelled L4 GPIO/RCC.
const STM32L4_BLINK_SOURCE = `#include <stdint.h>

#define RCC_AHB2ENR (*(volatile uint32_t *)0x4002104Cu)
#define GPIOA_MODER (*(volatile uint32_t *)0x48000000u)
#define GPIOA_ODR   (*(volatile uint32_t *)0x48000014u)

int main(void) {
  RCC_AHB2ENR |= (1u << 0u);                                    /* GPIOA clock */
  GPIOA_MODER = (GPIOA_MODER & ~(0x3u << 10u)) | (0x1u << 10u); /* PA5 output */
  while (1) {
    GPIOA_ODR ^= (1u << 5u);
    for (volatile uint32_t i = 0; i < 100000u; i++) {}
  }
}
`;

/** A runnable blink sketch for the diagram's board, so every shared lab runs. */
function defaultSourceForBoard(board: string): string {
  const b = board.toLowerCase();
  if (b.startsWith('stm32l4') || b.includes('l476')) return STM32L4_BLINK_SOURCE;
  return STM32F1_BLINK_SOURCE;
}

function starterSource(): string {
  return STM32F1_BLINK_SOURCE;
}

function diagramOrStarter(diagram: unknown): Record<string, unknown> {
  if (diagram && typeof diagram === 'object' && !Array.isArray(diagram)) {
    return normalizeLabWiredDiagramV1(diagram) as unknown as Record<string, unknown>;
  }
  return starterDiagram('stm32f103-blinky');
}

function sceneFromDiagram(diagram: Record<string, unknown>): Record<string, unknown> {
  return {
    board: typeof diagram.board === 'string' ? diagram.board : 'stm32l476',
    parts: Array.isArray(diagram.parts) ? diagram.parts : [],
    wires: Array.isArray(diagram.wires) ? diagram.wires : [],
    nets: Array.isArray(diagram.nets) ? diagram.nets : [],
  };
}

function boardChipForLabId(labId: string): string {
  return getPlaygroundBoard(labId)?.board ?? labId;
}

function starterDiagram(labId: string): Record<string, unknown> {
  const chip = boardChipForLabId(labId);
  return normalizeLabWiredDiagramV1({
    version: 1,
    board: chip,
    parts: [
      { id: 'mcu', type: 'mcu', label: chip.toUpperCase(), x: 180, y: 180, rotate: 0, attrs: {} },
      { id: 'led1', type: 'led', label: 'LED', x: 420, y: 180, rotate: 0, attrs: { color: 'green' } },
    ],
    wires: [
      { from: { part: 'mcu', pin: 'PA5' }, to: { part: 'led1', pin: 'A' }, color: '#3DD68C' },
    ],
  }) as unknown as Record<string, unknown>;
}
