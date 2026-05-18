#!/usr/bin/env node
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { z } from 'zod';
import { listChips, runSimulation, validateSystem, runLab } from './cli.js';
import { listBoards, getBoard, readBoardYamls } from './boards.js';
import { putSnapshot, getSnapshot } from './snapshots.js';

const SERVER_NAME = '@labwired/mcp';
const SERVER_VERSION = '0.2.0';

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

const InspectRunInput = z.object({
  snapshot_id: z.string().describe('snapshot_id returned by labwired_run_lab.'),
  scope: z
    .enum(['summary', 'serial', 'gpio', 'raw'])
    .default('summary')
    .describe('summary | serial | gpio | raw. Default summary.'),
});

const server = new Server(
  { name: SERVER_NAME, version: SERVER_VERSION },
  { capabilities: { tools: {} } },
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
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
        'Identical inputs always produce identical outputs (cycle-accurate). ' +
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
  ],
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;

  try {
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

      const run = await runLab({
        systemYaml: yamls.systemYaml,
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
