#!/usr/bin/env node
import { Server } from '@modelcontextprotocol/sdk/server/index.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from '@modelcontextprotocol/sdk/types.js';
import { z } from 'zod';
import { listChips, runSimulation, validateSystem } from './cli.js';

const SERVER_NAME = '@labwired/mcp';
const SERVER_VERSION = '0.1.1';

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
