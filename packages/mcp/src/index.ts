#!/usr/bin/env node
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
  ListResourcesRequestSchema,
  ReadResourceRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { z } from 'zod';
import { listChips, runSimulation, validateSystem, runLab, fuzzFirmware } from './cli.js';
import {
  listBoards,
  getBoard,
  readBoardYamls,
  boardSystemYamlPath,
  boardChipYamlPath,
} from './boards.js';
import { putSnapshot, getSnapshot } from './snapshots.js';
import { diagnoseDiagram, type ValidateDiagram } from './diagnostics.js';
import { SEARCH_TOOLS_TOOL, SEARCH_TOOLS_TOOL_NAME, rankTools } from './search-tools.js';
import { decorateTools } from './tool-metadata.js';
import { RESOURCES, getResource } from './resources.js';

const SERVER_NAME = '@labwired/mcp';
const SERVER_VERSION = '0.5.0';

// ─── Session API (Worker-backed) ───────────────────────────────────────────
// Optional: agent calls labwired_create_session(), gets a watch URL, and any
// subsequent run_lab / set_diagram / set_source updates the session's state on
// the Worker, which broadcasts to playground watchers via WebSocket.
const SESSIONS_API_BASE = process.env.LABWIRED_API ?? 'https://api.labwired.com';
const sessionStore: { id?: string; owner_token?: string; watch_url?: string } = {};

async function createWorkerSession(): Promise<{ session_id: string; owner_token: string; watch_url: string }> {
  const resp = await fetch(`${SESSIONS_API_BASE}/v1/sessions`, { method: 'POST' });
  if (!resp.ok) throw new Error(`session create failed: ${resp.status} ${await resp.text()}`);
  return (await resp.json()) as { session_id: string; owner_token: string; watch_url: string };
}

async function patchSession(diff: Record<string, unknown>): Promise<void> {
  if (!sessionStore.id || !sessionStore.owner_token) return;
  await fetch(`${SESSIONS_API_BASE}/v1/sessions/${sessionStore.id}`, {
    method: 'PUT',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${sessionStore.owner_token}`,
    },
    body: JSON.stringify(diff),
  }).catch(() => { /* best-effort */ });
}

const CatalogInput = z.object({
  filter: z
    .string()
    .optional()
    .describe('Optional substring filter applied to chip/board names.'),
});

const SimulateInput = z.object({
  firmware_base64: z
    .string()
    .describe('Base64-encoded ELF firmware image to load into the simulator.'),
  system_yaml: z
    .string()
    .describe(
      'Full contents of the System Manifest YAML (chip, peripherals, memory map). ' +
        'See https://github.com/w1ne/labwired for schema.',
    ),
  script_yaml: z
    .string()
    .describe(
      'Test script YAML: max_cycles, breakpoints, assertions on UART/registers/memory. ' +
        'Example schema: core/docs/ci_test_runner.md.',
    ),
  max_cycles: z
    .number()
    .int()
    .positive()
    .optional()
    .describe('Optional override for max cycles (takes precedence over script value).'),
});

const ValidateInput = z.object({
  system_yaml: z.string().describe('Full contents of the System Manifest YAML to validate.'),
});

const ListBoardsInput = z.object({
  filter: z.string().optional().describe('Substring filter on id / name / chip family.'),
});

const RunLabInput = z.object({
  board_id: z
    .string()
    .describe('Board id from labwired_list_boards. e.g. "stm32f103-blinky".'),
  elf_base64: z
    .string()
    .describe('Base64-encoded ELF to run. Agent compiles locally and uploads.'),
  max_cycles: z
    .number()
    .int()
    .positive()
    .max(100_000_000)
    .optional()
    .describe('Cycle budget (default 10M, hard cap 100M).'),
});

const FuzzInput = z.object({
  board_id: z
    .string()
    .describe('Board id from labwired_list_boards (resolves chip + system YAML).'),
  elf_base64: z
    .string()
    .describe(
      'Base64-encoded fuzz-target ELF. Must follow the fuzz contract: read input ' +
        'length+bytes from RAM, write DONE/FAULT to a verdict word. Agent compiles locally.',
    ),
  max_iters: z.number().int().positive().max(10_000_000).optional()
    .describe('Max fuzzing iterations (default 200k).'),
  seed: z.number().int().optional().describe('RNG seed — fuzzing is deterministic for a fixed seed.'),
  collect: z.number().int().positive().max(64).optional()
    .describe('Collect up to N distinct crashes (default 8).'),
  seed_inputs_hex: z.array(z.string()).optional()
    .describe('Seed inputs as hex byte strings, e.g. ["5000"]. Optional.'),
  contract: z
    .object({
      input_len_addr: z.string().optional(),
      input_data_addr: z.string().optional(),
      verdict_addr: z.string().optional(),
      done_magic: z.string().optional(),
      fault_magic: z.string().optional(),
    })
    .optional()
    .describe('Override the fuzz contract addresses/markers (hex). Defaults match the F103 fuzz target.'),
});

const InspectRunInput = z.object({
  snapshot_id: z.string().describe('snapshot_id returned by labwired_run_lab.'),
  scope: z
    .enum(['summary', 'serial', 'gpio', 'raw'])
    .default('summary')
    .describe('summary | serial | gpio | raw. Default summary.'),
});

const CreateSessionInput = z.object({});
const EndSessionInput = z.object({});

const SetDiagramInput = z.object({
  diagram: z.unknown().describe('Diagram JSON to mirror to the watch session.'),
});
const SetSourceInput = z.object({
  source: z.string().describe('Source code to mirror to the watch session.'),
});

const ValidateDiagramInput = z.object({
  diagram: z
    .object({
      board: z.string(),
      parts: z.array(z.object({ id: z.string(), type: z.string() }).passthrough()),
      wires: z.array(
        z.object({
          from: z.object({ part: z.string(), pin: z.string() }),
          to: z.object({ part: z.string(), pin: z.string() }),
        }),
      ),
    })
    .passthrough()
    .describe('Diagram JSON: { board, parts: [{id, type, ...}], wires: [{from, to}] }.'),
});

const server = new Server(
  { name: SERVER_NAME, version: SERVER_VERSION },
  { capabilities: { tools: {}, resources: {} } },
);

function localTools() {
  return [
    SEARCH_TOOLS_TOOL,
    {
      name: 'labwired_catalog',
      description:
        "List supported chips/boards in the LabWired catalog. Returns name, family, " +
        'architecture, pass rate, and verification status. Use this to discover what targets ' +
        'are available before running a simulation. Free; no API key required.',
      inputSchema: {
        type: 'object',
        properties: {
          filter: {
            type: 'string',
            description: 'Optional substring filter applied to chip/board names.',
          },
        },
      },
    },
    {
      name: 'labwired_simulate',
      description:
        'Run firmware against the LabWired deterministic simulator. ' +
        'Identical inputs always produce identical outputs (deterministic, silicon-validated). ' +
        'Returns result.json (assertions, exit status, cycles consumed) and the captured UART log. ' +
        'Use this to verify firmware behaviour, reproduce bugs, or check whether a fix works.',
      inputSchema: {
        type: 'object',
        required: ['firmware_base64', 'system_yaml', 'script_yaml'],
        properties: {
          firmware_base64: {
            type: 'string',
            description: 'Base64-encoded ELF firmware image to load into the simulator.',
          },
          system_yaml: {
            type: 'string',
            description:
              'System Manifest YAML defining chip, peripherals, memory map. ' +
              'See github.com/w1ne/labwired for schema.',
          },
          script_yaml: {
            type: 'string',
            description:
              'Test script YAML: max_cycles, breakpoints, assertions on UART / registers / memory.',
          },
          max_cycles: {
            type: 'integer',
            description: 'Optional override for max cycles.',
          },
        },
      },
    },
    {
      name: 'labwired_validate_system',
      description:
        'Validate a System Manifest YAML against the LabWired schema and confirm referenced ' +
        'chip descriptors exist. Returns the validator stdout / stderr. ' +
        'Use this before simulate() to catch schema errors fast.',
      inputSchema: {
        type: 'object',
        required: ['system_yaml'],
        properties: {
          system_yaml: {
            type: 'string',
            description: 'Full contents of the System Manifest YAML to validate.',
          },
        },
      },
    },
    {
      name: 'labwired_list_boards',
      description:
        'List supported boards (chip + pre-wired peripherals + demo firmware). Higher-level ' +
        'than labwired_catalog: each board has a complete simulation config you can run with ' +
        'labwired_run_lab. Returns id, name, chip family, arch, and description for each.',
      inputSchema: {
        type: 'object',
        properties: {
          filter: {
            type: 'string',
            description: 'Substring filter on id / name / chip family.',
          },
        },
      },
    },
    {
      name: 'labwired_run_lab',
      description:
        'Run a firmware ELF against a pre-configured board. Pick a board with labwired_list_boards, ' +
        "compile your firmware locally (we don't compile for you), then upload the ELF here. " +
        'Returns final cycles, exit reason, UART serial output, and a snapshot_id you can pass to ' +
        'labwired_inspect_run for deeper state. Deterministic: identical inputs → identical output.',
      inputSchema: {
        type: 'object',
        required: ['board_id', 'elf_base64'],
        properties: {
          board_id: {
            type: 'string',
            description: 'Board id from labwired_list_boards (e.g. "stm32f103-blinky").',
          },
          elf_base64: {
            type: 'string',
            description: 'Base64-encoded ELF binary. Agent compiles locally.',
          },
          max_cycles: {
            type: 'integer',
            description: 'Cycle budget (default 10M, hard cap 100M).',
          },
        },
      },
    },
    {
      name: 'labwired_fuzz',
      description:
        'Coverage-guided fuzz a firmware ELF in the silicon-validated simulator and return the ' +
        'crashing inputs. AFL-style edge coverage drives mutation; a crash is a CPU fault or the ' +
        "firmware's FAULT marker. Because the sim is silicon-validated, crashes found here are " +
        'replayable on real hardware (HIL-confirm) — silicon-true findings, not emulation false ' +
        'positives. The target firmware must follow the fuzz contract (RAM length+data input ' +
        'buffer, a verdict word with DONE/FAULT markers). Deterministic for a fixed seed. Returns ' +
        'the distinct crashing inputs (as byte arrays) you can replay or minimize.',
      inputSchema: {
        type: 'object',
        required: ['board_id', 'elf_base64'],
        properties: {
          board_id: {
            type: 'string',
            description: 'Board id from labwired_list_boards (resolves chip + system YAML).',
          },
          elf_base64: {
            type: 'string',
            description:
              'Base64-encoded fuzz-target ELF following the fuzz contract. Agent compiles locally.',
          },
          max_iters: {
            type: 'integer',
            description: 'Max fuzzing iterations (default 200000).',
          },
          seed: {
            type: 'integer',
            description: 'RNG seed — fuzzing is deterministic for a fixed seed.',
          },
          collect: {
            type: 'integer',
            description: 'Collect up to N distinct crashes (default 8).',
          },
          seed_inputs_hex: {
            type: 'array',
            items: { type: 'string' },
            description: 'Seed inputs as hex byte strings, e.g. ["5000"].',
          },
          contract: {
            type: 'object',
            description:
              'Override the fuzz contract (hex strings). Defaults match the F103 fuzz target: ' +
              'input_len_addr 0x20002800, input_data_addr 0x20002804, verdict_addr 0x20003000, ' +
              'done_magic 0xC0DEF022, fault_magic 0xDEADFA17.',
            properties: {
              input_len_addr: { type: 'string' },
              input_data_addr: { type: 'string' },
              verdict_addr: { type: 'string' },
              done_magic: { type: 'string' },
              fault_magic: { type: 'string' },
            },
          },
        },
      },
    },
    {
      name: 'labwired_inspect_run',
      description:
        'Retrieve a deeper slice of state from a prior labwired_run_lab snapshot. Use scope=summary ' +
        'for cycles + PC + exit reason, scope=serial for full UART transcript, scope=gpio for pin ' +
        'transitions (when CLI emits them), scope=raw for the underlying result.json. Snapshots ' +
        'expire after 10 min.',
      inputSchema: {
        type: 'object',
        required: ['snapshot_id'],
        properties: {
          snapshot_id: {
            type: 'string',
            description: 'snapshot_id from labwired_run_lab output.',
          },
          scope: {
            type: 'string',
            enum: ['summary', 'serial', 'gpio', 'raw'],
            description: 'Slice to return. Default summary.',
          },
        },
      },
    },
    {
      name: 'labwired_create_session',
      description:
        'Create a live watch session. Returns a watch_url like ' +
        'https://app.labwired.com/?watch=<id> that a human can open to see your runs ' +
        'unfold in real time — diagram, source, and simulation state stream to the browser ' +
        'via WebSocket. After this call, subsequent labwired_run_lab / labwired_set_diagram / ' +
        'labwired_set_source calls automatically mirror to the session. Anonymous; the URL is ' +
        'the only credential.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'labwired_end_session',
      description:
        'End the active watch session. Subsequent tool calls no longer mirror state to a watcher.',
      inputSchema: { type: 'object', properties: {} },
    },
    {
      name: 'labwired_set_diagram',
      description:
        "Push a diagram (parts + wires) to the active watch session so a human watcher can see " +
        "the circuit you're building. No-op if no session is active.",
      inputSchema: {
        type: 'object',
        required: ['diagram'],
        properties: { diagram: { type: 'object' } },
      },
    },
    {
      name: 'labwired_set_source',
      description:
        "Push source code to the active watch session. Mirrors what your agent is writing so a " +
        "human can read along. No-op if no session is active.",
      inputSchema: {
        type: 'object',
        required: ['source'],
        properties: { source: { type: 'string' } },
      },
    },
    {
      name: 'labwired_validate_diagram',
      description:
        'Structurally validate a wired diagram BEFORE attempting to run it. Returns an array of ' +
        'machine-readable diagnostics (severity, code, message, location, suggested fix) — much ' +
        'friendlier than waiting for the simulator to fail at run time. Common codes: ' +
        'PIN_NOT_ON_CHIP, PIN_LACKS_I2C, BOARDIO_NOT_TO_MCU, NO_MCU, COMPONENT_DANGLING. Use this ' +
        'when building circuits programmatically; iterate until you get back an empty array.',
      inputSchema: {
        type: 'object',
        required: ['diagram'],
        properties: {
          diagram: {
            type: 'object',
            description:
              'Diagram JSON: { board: "stm32f103", parts: [{id, type}], wires: [{from:{part,pin}, to:{part,pin}}] }',
          },
        },
      },
    },
  ];
}

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: decorateTools(localTools()),
}));

server.setRequestHandler(ListResourcesRequestSchema, async () => ({
  resources: RESOURCES,
}));

server.setRequestHandler(ReadResourceRequestSchema, async (request) => {
  const resource = getResource(request.params.uri);
  if (!resource) throw new Error(`Unknown resource: ${request.params.uri}`);
  return { contents: [resource] };
});

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
    if (name === SEARCH_TOOLS_TOOL_NAME) {
      const input = (args ?? {}) as { query?: unknown; limit?: unknown };
      const query = typeof input.query === 'string' ? input.query : '';
      const limit = typeof input.limit === 'number' && Number.isFinite(input.limit)
        ? Math.trunc(input.limit)
        : 8;
      return {
        content: [{ type: 'text', text: JSON.stringify({ query, tools: rankTools(query, decorateTools(localTools()), limit) }) }],
      };
    }

    if (name === 'labwired_catalog') {
      const { filter } = CatalogInput.parse(args ?? {});
      const chips = await listChips(filter);
      return {
        content: [{ type: 'text', text: JSON.stringify(chips, null, 2) }],
      };
    }

    if (name === 'labwired_simulate') {
      const input = SimulateInput.parse(args ?? {});
      const run = await runSimulation({
        firmwareBase64: input.firmware_base64,
        systemYaml: input.system_yaml,
        scriptYaml: input.script_yaml,
        maxCycles: input.max_cycles,
      });
      const summary = {
        exit_code: run.exitCode,
        result: run.resultJson,
        uart_log_excerpt: run.uartLog.slice(0, 4000),
        uart_log_truncated: run.uartLog.length > 4000,
        stderr_excerpt: run.stderr.slice(0, 2000),
      };
      return {
        content: [{ type: 'text', text: JSON.stringify(summary, null, 2) }],
        isError: run.exitCode !== 0,
      };
    }

    if (name === 'labwired_validate_system') {
      const { system_yaml } = ValidateInput.parse(args ?? {});
      const result = await validateSystem(system_yaml);
      return {
        content: [
          {
            type: 'text',
            text: JSON.stringify(
              { exit_code: result.exitCode, stdout: result.stdout, stderr: result.stderr },
              null,
              2,
            ),
          },
        ],
        isError: result.exitCode !== 0,
      };
    }

    if (name === 'labwired_list_boards') {
      const { filter } = ListBoardsInput.parse(args ?? {});
      const boards = listBoards(filter);
      return {
        content: [{ type: 'text', text: JSON.stringify({ boards }, null, 2) }],
      };
    }

    if (name === 'labwired_run_lab') {
      const input = RunLabInput.parse(args ?? {});
      const board = getBoard(input.board_id);
      if (!board) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  error: 'INVALID_BOARD',
                  message: `Unknown board_id "${input.board_id}". Call labwired_list_boards to see available ids.`,
                },
                null,
                2,
              ),
            },
          ],
          isError: true,
        };
      }

      let yamls;
      try {
        yamls = await readBoardYamls(board);
      } catch (e) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  error: 'BOARD_YAMLS_UNAVAILABLE',
                  message: e instanceof Error ? e.message : String(e),
                  hint: 'The MCP server needs the labwired repo on disk to resolve board YAMLs. Set LABWIRED_REPO_ROOT if running outside the repo.',
                },
                null,
                2,
              ),
            },
          ],
          isError: true,
        };
      }

      const firmware = Buffer.from(input.elf_base64, 'base64');
      if (firmware.length < 4 || firmware.subarray(0, 4).toString('hex') !== '7f454c46') {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  error: 'INVALID_ELF',
                  message: 'Decoded firmware is not an ELF file (missing 0x7F454C46 magic).',
                },
                null,
                2,
              ),
            },
          ],
          isError: true,
        };
      }

      // Use absolute repo path for systemYaml so relative refs
      // (chip:, descriptor:) inside it resolve to core/configs/...
      const run = await runLab({
        systemYamlPath: boardSystemYamlPath(board),
        firmware,
        maxCycles: input.max_cycles,
      });

      const snapshot_id = putSnapshot({
        registers: undefined,
        serial_output: run.serial_output,
        gpio_events: run.gpio_events,
        gpio_truncated: run.gpio_truncated,
        final_pc_hex: run.final_pc_hex,
        final_cycles: run.final_cycles,
        raw_result: run.raw_result,
        board_id: input.board_id,
        created_at: Date.now(),
      });

      // Mirror to the active watch session so humans see it live (best-effort).
      await patchSession({
        board_id: input.board_id,
        status: run.exit_code === 0 ? 'completed' : 'failed',
        last_sim_state: {
          exit_reason: run.exit_reason,
          final_cycles: run.final_cycles,
          final_pc_hex: run.final_pc_hex,
          serial_tail: run.serial_output.slice(-4000),
          snapshot_id,
        },
      });

      // Trim serial in the inline response — full transcript is in snapshot.
      const SERIAL_INLINE_CAP = 8000;
      const serialInline = run.serial_output.slice(-SERIAL_INLINE_CAP);

      return {
        content: [
          {
            type: 'text',
            text: JSON.stringify(
              {
                ok: run.exit_code === 0,
                snapshot_id,
                board_id: input.board_id,
                exit_reason: run.exit_reason,
                final_cycles: run.final_cycles,
                final_pc_hex: run.final_pc_hex,
                serial_tail: serialInline,
                serial_truncated_in_response:
                  run.serial_output.length > SERIAL_INLINE_CAP || run.serial_truncated,
                stderr_excerpt: run.stderr.slice(0, 2000),
              },
              null,
              2,
            ),
          },
        ],
        isError: run.exit_code !== 0,
      };
    }

    if (name === 'labwired_fuzz') {
      const input = FuzzInput.parse(args ?? {});
      const board = getBoard(input.board_id);
      if (!board) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  error: 'INVALID_BOARD',
                  message: `Unknown board_id "${input.board_id}". Call labwired_list_boards to see available ids.`,
                },
                null,
                2,
              ),
            },
          ],
          isError: true,
        };
      }

      const firmware = Buffer.from(input.elf_base64, 'base64');
      if (firmware.length < 4 || firmware.subarray(0, 4).toString('hex') !== '7f454c46') {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  error: 'INVALID_ELF',
                  message: 'Decoded firmware is not an ELF file (missing 0x7F454C46 magic).',
                },
                null,
                2,
              ),
            },
          ],
          isError: true,
        };
      }

      let run;
      try {
        run = await fuzzFirmware({
          chipYamlPath: boardChipYamlPath(board),
          systemYamlPath: boardSystemYamlPath(board),
          firmware,
          maxIters: input.max_iters,
          seed: input.seed,
          collect: input.collect,
          seedInputsHex: input.seed_inputs_hex,
          contract: input.contract && {
            inputLenAddr: input.contract.input_len_addr,
            inputDataAddr: input.contract.input_data_addr,
            verdictAddr: input.contract.verdict_addr,
            doneMagic: input.contract.done_magic,
            faultMagic: input.contract.fault_magic,
          },
        });
      } catch (e) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  error: 'FUZZ_FAILED',
                  message: e instanceof Error ? e.message : String(e),
                  hint: 'The MCP server needs the labwired repo on disk to resolve board YAMLs. Set LABWIRED_REPO_ROOT.',
                },
                null,
                2,
              ),
            },
          ],
          isError: true,
        };
      }

      const summary = {
        crashed: run.crashed,
        crash_count: run.crashes.length,
        // Crashes as hex strings for readability + raw byte arrays for replay.
        crashes_hex: run.crashes.map((c) =>
          c.map((b) => b.toString(16).padStart(2, '0')).join(''),
        ),
        crashes: run.crashes,
        note: run.crashed
          ? 'Crashes are silicon-true: replay any input on the F103 via the HIL-confirm harness to confirm.'
          : 'No crash found within the iteration budget.',
        stdout_excerpt: run.stdout.slice(0, 2000),
        stderr_excerpt: run.stderr.slice(0, 2000),
      };
      return {
        content: [{ type: 'text', text: JSON.stringify(summary, null, 2) }],
        // A found crash is a finding, not a tool error — surface it as content,
        // not isError (which signals the tool itself failed).
        isError: false,
      };
    }

    if (name === 'labwired_create_session') {
      CreateSessionInput.parse(args ?? {});
      const s = await createWorkerSession();
      sessionStore.id = s.session_id;
      sessionStore.owner_token = s.owner_token;
      sessionStore.watch_url = s.watch_url;
      return {
        content: [
          {
            type: 'text',
            text: JSON.stringify(
              {
                ok: true,
                session_id: s.session_id,
                watch_url: s.watch_url,
                hint: 'Open the watch_url in a browser to watch this session live. Subsequent run_lab / set_diagram / set_source mirror automatically.',
              },
              null,
              2,
            ),
          },
        ],
      };
    }

    if (name === 'labwired_end_session') {
      EndSessionInput.parse(args ?? {});
      const wasActive = !!sessionStore.id;
      if (sessionStore.id && sessionStore.owner_token) {
        await fetch(`${SESSIONS_API_BASE}/v1/sessions/${sessionStore.id}`, {
          method: 'DELETE',
          headers: { Authorization: `Bearer ${sessionStore.owner_token}` },
        }).catch(() => { /* best-effort */ });
      }
      sessionStore.id = undefined;
      sessionStore.owner_token = undefined;
      sessionStore.watch_url = undefined;
      return {
        content: [{ type: 'text', text: JSON.stringify({ ok: true, was_active: wasActive }) }],
      };
    }

    if (name === 'labwired_set_diagram') {
      const { diagram } = SetDiagramInput.parse(args ?? {});
      if (!sessionStore.id) {
        return {
          content: [
            { type: 'text', text: JSON.stringify({ ok: false, error: 'NO_ACTIVE_SESSION', hint: 'Call labwired_create_session first.' }) },
          ],
          isError: true,
        };
      }
      await patchSession({ diagram });
      return { content: [{ type: 'text', text: JSON.stringify({ ok: true, mirrored: true }) }] };
    }

    if (name === 'labwired_set_source') {
      const { source } = SetSourceInput.parse(args ?? {});
      if (!sessionStore.id) {
        return {
          content: [
            { type: 'text', text: JSON.stringify({ ok: false, error: 'NO_ACTIVE_SESSION', hint: 'Call labwired_create_session first.' }) },
          ],
          isError: true,
        };
      }
      await patchSession({ source });
      return { content: [{ type: 'text', text: JSON.stringify({ ok: true, mirrored: true }) }] };
    }

    if (name === 'labwired_validate_diagram') {
      const { diagram } = ValidateDiagramInput.parse(args ?? {});
      const diagnostics = diagnoseDiagram(diagram as unknown as ValidateDiagram);
      const errors = diagnostics.filter((d) => d.severity === 'error').length;
      const warnings = diagnostics.filter((d) => d.severity === 'warning').length;
      return {
        content: [
          {
            type: 'text',
            text: JSON.stringify(
              {
                ok: errors === 0,
                error_count: errors,
                warning_count: warnings,
                diagnostics,
              },
              null,
              2,
            ),
          },
        ],
        isError: errors > 0,
      };
    }

    if (name === 'labwired_inspect_run') {
      const { snapshot_id, scope } = InspectRunInput.parse(args ?? {});
      const snap = getSnapshot(snapshot_id);
      if (!snap) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  error: 'SNAPSHOT_EXPIRED',
                  message: `Snapshot ${snapshot_id} not found. TTL is 10 minutes; re-run labwired_run_lab.`,
                },
                null,
                2,
              ),
            },
          ],
          isError: true,
        };
      }
      const payload: Record<string, unknown> = { snapshot_id, scope, board_id: snap.board_id };
      if (scope === 'summary' || scope === 'raw') {
        payload.final_cycles = snap.final_cycles;
        payload.final_pc_hex = snap.final_pc_hex;
      }
      if (scope === 'serial' || scope === 'summary') {
        payload.serial_output = snap.serial_output ?? '';
      }
      if (scope === 'gpio') {
        payload.gpio_events = snap.gpio_events ?? [];
        payload.gpio_truncated = snap.gpio_truncated ?? false;
      }
      if (scope === 'raw') {
        payload.raw_result = snap.raw_result;
      }
      return { content: [{ type: 'text', text: JSON.stringify(payload, null, 2) }] };
    }

    return {
      content: [{ type: 'text', text: `Unknown tool: ${name}` }],
      isError: true,
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return {
      content: [{ type: 'text', text: `Tool error: ${message}` }],
      isError: true,
    };
  }
});

const transport = new StdioServerTransport();
await server.connect(transport);
