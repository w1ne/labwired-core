import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { rm } from 'node:fs/promises';
import { chmodSync, existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';


const DIST = join(import.meta.dirname, '..', 'dist', 'index.js');
let mockCliDir: string | undefined;

beforeAll(() => {
  mockCliDir = mkdtempSync(join(tmpdir(), 'labwired-mcp-cli-'));
  const mockCli = join(mockCliDir, 'labwired');
  writeFileSync(
    mockCli,
    `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const args = process.argv.slice(2);

if (args[0] !== 'asset') process.exit(2);

if (args[1] === 'validate-component') {
  const spec = fs.readFileSync(args[2], 'utf8');
  if (/kind:\\s*wasm/.test(spec)) {
    console.log(JSON.stringify({ ok: false, name: null, diagnostics: [{ code: 'ICOMP_WASM_UNSUPPORTED' }] }));
  } else {
    const name = spec.match(/^name:\\s*([^\\n]+)/m)?.[1]?.trim() ?? 'Component';
    console.log(JSON.stringify({ ok: true, name, diagnostics: [] }));
  }
  process.exit(0);
}

if (args[1] === 'ingest-svd') {
  const outputDir = args[args.indexOf('--output-dir') + 1];
  fs.mkdirSync(outputDir, { recursive: true });
  const descriptor = path.resolve(outputDir, 'gpioa.yaml');
  fs.writeFileSync(descriptor, [
    'peripheral: GPIOA',
    'base_address: 0x40010800',
    'registers:',
    '  - name: CRL',
    '    offset: 0x00',
    '',
  ].join('\\n'));
  console.log(JSON.stringify({
    output_dir: outputDir,
    peripheral_count: 1,
    peripherals: [{ name: 'GPIOA', descriptor_path: descriptor, register_count: 1, base_address: '0x40010800' }],
  }));
  process.exit(0);
}

process.exit(2);
`,
  );
  chmodSync(mockCli, 0o755);
  process.env.LABWIRED_CLI = mockCli;
});

afterAll(() => {
  if (mockCliDir) {
    rmSync(mockCliDir, { recursive: true, force: true });
  }
});

const TEST_DEVICE_SVD = `<?xml version="1.0" encoding="utf-8"?>
<device schemaVersion="1.3">
  <name>STM32F103</name>
  <version>1.0</version>
  <description>STM32F103xx</description>
  <addressUnitBits>8</addressUnitBits>
  <width>32</width>
  <size>32</size>
  <resetValue>0x00000000</resetValue>
  <resetMask>0xFFFFFFFF</resetMask>
  <peripherals>
    <peripheral>
      <name>GPIOA</name>
      <baseAddress>0x40010800</baseAddress>
      <registers>
        <register>
          <name>CRL</name>
          <addressOffset>0x00</addressOffset>
          <size>32</size>
          <fields>
            <field>
              <name>MODE0</name>
              <bitOffset>0</bitOffset>
              <bitWidth>2</bitWidth>
            </field>
          </fields>
        </register>
      </registers>
    </peripheral>
  </peripherals>
</device>`;

const INIT_MESSAGES = [
  {
    jsonrpc: '2.0',
    id: 1,
    method: 'initialize',
    params: {
      protocolVersion: '2025-03-26',
      capabilities: {},
      clientInfo: { name: 'smoketest', version: '0' },
    },
  },
  { jsonrpc: '2.0', method: 'notifications/initialized' },
];

function rpcRoundTrip(messages: object[]): Promise<string> {
  return new Promise((resolve, reject) => {
    const proc = spawn('node', [DIST], { stdio: ['pipe', 'pipe', 'pipe'] });
    let out = '';
    proc.stdout.on('data', (b: Buffer) => {
      out += b.toString();
    });
    proc.on('error', reject);
    proc.on('exit', () => resolve(out));
    for (const m of messages) proc.stdin.write(JSON.stringify(m) + '\n');
    setTimeout(() => proc.kill(), 3000);
  });
}

async function listToolsViaStdio(): Promise<any[]> {
  const out = await rpcRoundTrip([
    ...INIT_MESSAGES,
    { jsonrpc: '2.0', id: 2, method: 'tools/list' },
  ]);
  for (const line of out.split('\n')) {
    try {
      const msg = JSON.parse(line);
      if (msg.id === 2 && msg.result?.tools) return msg.result.tools;
    } catch { /* skip non-JSON lines */ }
  }
  return [];
}

async function callToolViaStdio(
  toolName: string,
  args: Record<string, unknown>,
): Promise<{ isError: boolean; content: Array<{ type: string; text: string }> }> {
  const out = await rpcRoundTrip([
    ...INIT_MESSAGES,
    {
      jsonrpc: '2.0',
      id: 3,
      method: 'tools/call',
      params: { name: toolName, arguments: args },
    },
  ]);
  for (const line of out.split('\n')) {
    try {
      const msg = JSON.parse(line);
      if (msg.id === 3 && msg.result) return msg.result;
    } catch { /* skip non-JSON lines */ }
  }
  throw new Error(`No response for tools/call ${toolName}. Server output:\n${out}`);
}

describe('@labwired/mcp stdio server', () => {
  it('responds with annotated tools, search, and the agent guide resource', async () => {
    const out = await rpcRoundTrip([
      ...INIT_MESSAGES,
      { jsonrpc: '2.0', id: 2, method: 'tools/list' },
      { jsonrpc: '2.0', id: 3, method: 'resources/list' },
      {
        jsonrpc: '2.0',
        id: 4,
        method: 'resources/read',
        params: { uri: 'labwired://guides/agent-hardware-loop' },
      },
      {
        jsonrpc: '2.0',
        id: 5,
        method: 'tools/call',
        params: { name: 'labwired_search_tools', arguments: { query: 'diagram validation', limit: 3 } },
      },
    ]);
    expect(out).toContain('"name":"labwired_catalog"');
    expect(out).toContain('"name":"labwired_simulate"');
    expect(out).toContain('"name":"labwired_validate_system"');
    expect(out).toContain('"name":"labwired_search_tools"');
    expect(out).toContain('"title":"Run Lab"');
    expect(out).toContain('"annotations"');
    expect(out).toContain('"readOnlyHint":true');
    expect(out).toContain('"uri":"labwired://guides/agent-hardware-loop"');
    expect(out).toContain('LabWired agent hardware loop');
    expect(out).toContain('labwired_validate_diagram');
  });

  it('search returns guide, workflow, and callable annotations for agents', async () => {
    const result = await callToolViaStdio('labwired_search_tools', {
      query: 'build hardware run firmware inspect evidence',
      limit: 4,
    });
    expect(result.isError).toBeFalsy();
    const payload = JSON.parse(result.content[0].text);
    expect(payload.guide_uri).toBe('labwired://guides/agent-hardware-loop');
    expect(payload.workflow).toEqual([
      'labwired_list_boards',
      'labwired_validate_diagram',
      'labwired_compile_diagram',
      'labwired_simulate',
      'labwired_inspect_run',
    ]);
    expect(payload.tools[0].annotations).toMatchObject({
      destructiveHint: false,
    });
  });
});

describe('labwired_compile_diagram', () => {
  const repoRoot = process.env.LABWIRED_REPO_ROOT ?? join(import.meta.dirname, '..', '..', '..');
  const boardsDir = join(repoRoot, '.labwired', 'boards');

  afterEach(async () => {
    // Clean up persisted board artifacts
    for (const file of ['esp32-s3-zero.yaml', 'test-board.yaml']) {
      const artifact = join(boardsDir, file);
      if (existsSync(artifact)) {
        await rm(artifact, { force: true });
      }
    }
  });

  it('is advertised with title "Compile Diagram" and readOnlyHint false', async () => {
    const tools = await listToolsViaStdio();
    const tool = tools.find((t: any) => t.name === 'labwired_compile_diagram');
    expect(tool).toBeDefined();
    expect(tool.title).toBe('Compile Diagram');
    expect(tool.annotations.readOnlyHint).toBe(false);
  });

  it('returns isError for a diagram with an unknown component type', async () => {
    const result = await callToolViaStdio('labwired_compile_diagram', {
      diagram: {
        board: 'esp32-s3-zero',
        parts: [
          { id: 'mcu', type: 'esp32-s3-zero' },
          { id: 'bad1', type: 'totally_unknown_part_xyz' },
        ],
        wires: [],
      },
    });
    // UNKNOWN_COMPONENT is an error that should abort compile
    expect(result.isError).toBe(true);
  });

  it('compiles a clean dispenser diagram → ok, board_path, i2c in yaml', async () => {
    const result = await callToolViaStdio('labwired_compile_diagram', {
      diagram: {
        board: 'esp32-s3-zero',
        parts: [
          { id: 'mcu', type: 'esp32-s3-zero' },
          { id: 'pca1', type: 'pca9685', attrs: { i2c_address: '0x40' } },
        ],
        wires: [
          { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca1', pin: 'SDA' } },
          { from: { part: 'mcu', pin: 'GPIO9' }, to: { part: 'pca1', pin: 'SCL' } },
          { from: { part: 'mcu', pin: '3V3' }, to: { part: 'pca1', pin: 'VCC' } },
          { from: { part: 'mcu', pin: 'GND' }, to: { part: 'pca1', pin: 'GND' } },
        ],
      },
    });
    expect(result.isError).toBeFalsy();
    const body = JSON.parse(result.content[0].text);
    expect(body.ok).toBe(true);
    expect(body.board_path).toMatch(/\.labwired\/boards\/.*\.yaml$/);
    expect(body.system_yaml).toContain('i2c');
  });
});

describe('labwired_validate_diagram (kernel codes)', () => {
  it('reports kernel I2C_ADDR_CONFLICT when two devices share the same address', async () => {
    // Two pca9685 at same address 0x40 on same bus
    const result = await callToolViaStdio('labwired_validate_diagram', {
      diagram: {
        board: 'esp32-s3-zero',
        parts: [
          { id: 'mcu', type: 'esp32-s3-zero' },
          { id: 'pca1', type: 'pca9685', attrs: { i2c_address: '0x40' } },
          { id: 'pca2', type: 'pca9685', attrs: { i2c_address: '0x40' } },
        ],
        wires: [
          { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca1', pin: 'SDA' } },
          { from: { part: 'mcu', pin: 'GPIO9' }, to: { part: 'pca1', pin: 'SCL' } },
          { from: { part: 'mcu', pin: 'GPIO8' }, to: { part: 'pca2', pin: 'SDA' } },
          { from: { part: 'mcu', pin: 'GPIO9' }, to: { part: 'pca2', pin: 'SCL' } },
          { from: { part: 'mcu', pin: '3V3' }, to: { part: 'pca1', pin: 'VCC' } },
          { from: { part: 'mcu', pin: 'GND' }, to: { part: 'pca1', pin: 'GND' } },
          { from: { part: 'mcu', pin: '3V3' }, to: { part: 'pca2', pin: 'VCC' } },
          { from: { part: 'mcu', pin: 'GND' }, to: { part: 'pca2', pin: 'GND' } },
        ],
      },
    });
    const body = JSON.parse(result.content[0].text);
    const codes = (body.diagnostics as any[]).map((d: any) => d.code);
    expect(codes).toContain('I2C_ADDR_CONFLICT');
  });
});

describe('labwired_define_component', () => {
  const repoRoot = process.env.LABWIRED_REPO_ROOT ?? join(import.meta.dirname, '..', '..', '..');
  const componentDir = join(repoRoot, '.labwired', 'components');

  afterEach(async () => {
    // Clean up any persisted test component
    const artifact = join(componentDir, 'tca9999.yaml');
    if (existsSync(artifact)) {
      await rm(artifact, { force: true });
    }
  });

  it('is advertised with title and annotations', async () => {
    const tools = await listToolsViaStdio();
    const tool = tools.find((t: any) => t.name === 'labwired_define_component');
    expect(tool).toBeDefined();
    expect(tool.title).toBe('Define Component');
    expect(tool.annotations.readOnlyHint).toBe(false);
  });

  it('rejects an invalid spec with machine-readable diagnostics', async () => {
    const result = await callToolViaStdio('labwired_define_component', {
      spec_yaml:
        'name: Bad\nkind: wasm\ninterface: { i2c: { default_address: 0x40 } }\nregister_file: { size: 256 }\n',
    });
    expect(result.isError).toBe(true);
    expect(JSON.stringify(result.content)).toContain('ICOMP_WASM_UNSUPPORTED');
  });

  it('persists a valid spec and returns spec_path + manifest usage', async () => {
    const specYaml = [
      'name: TCA9999',
      'kind: declarative',
      'interface: { i2c: { default_address: 0x20 } }',
      'register_file: { size: 8 }',
    ].join('\n');
    const result = await callToolViaStdio('labwired_define_component', {
      spec_yaml: specYaml,
    });
    expect(result.isError).toBeFalsy();
    const body = JSON.parse(result.content[0].text);
    expect(body.ok).toBe(true);
    expect(body.spec_path).toMatch(/\.labwired\/components\/tca9999\.yaml$/);
    expect(body.usage.manifest_external_device.type).toBe('ir');
  });
});

describe('labwired_ingest_svd', () => {
  const repoRoot = process.env.LABWIRED_REPO_ROOT ?? join(import.meta.dirname, '..', '..', '..');
  const peripheralDir = join(repoRoot, '.labwired', 'peripherals');

  afterEach(async () => {
    const artifact = join(peripheralDir, 'gpioa.yaml');
    if (existsSync(artifact)) {
      await rm(artifact, { force: true });
    }
  });

  it('is advertised in tools/list', async () => {
    const tools = await listToolsViaStdio();
    expect(tools.find((t: any) => t.name === 'labwired_ingest_svd')).toBeDefined();
  });

  it('ingests an SVD into descriptors + a paste-ready declarative snippet', async () => {
    const result = await callToolViaStdio('labwired_ingest_svd', { svd_content: TEST_DEVICE_SVD });
    expect(result.isError).toBeFalsy();
    const body = JSON.parse(result.content[0].text);
    expect(body.ok).toBe(true);
    expect(body.peripheral_count).toBe(1);
    expect(body.peripherals[0].name).toBe('GPIOA');
    expect(body.peripherals[0].descriptor_yaml).toContain('peripheral: GPIOA');
    expect(body.peripherals[0].base_address).toMatch(/^0x[0-9A-F]{8}$/);
    // Paste-ready chip-yaml block.
    expect(body.manifest_snippet).toContain('type: declarative');
    expect(body.manifest_snippet).toContain('base_address:');
    expect(body.manifest_snippet).toContain('- id: gpioa');
  });
});
