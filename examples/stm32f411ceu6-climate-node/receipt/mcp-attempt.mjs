// Attempt the MCP stdio `labwired_run` / `labwired_verify` tools against the
// climate node and archive the raw responses to receipt/mcp-stdio-attempt.json.
//
// Heavier than receipt.sh: it spawns the real @labwired/mcp stdio server from
// the monorepo (packages/mcp/src/index.ts via tsx), so the monorepo
// node_modules must be installed and
//   * packages/kicad-monkey/dist  built (tsc -p tsconfig.json)
//   * packages/board-config/dist  built (esbuild; the registry the MCP reads)
//
// Env:
//   LABWIRED_REPO_ROOT   monorepo root (default: ../../../.. from this file's dir)
//   LABWIRED_CLI         engine binary (default <root>/core/target/release/labwired)
//
// It also seeds the documented local stdio artifact cache
// (~/.labwired/artifacts/sha256/<hex>; firmware-ref.ts resolves it first) so
// the local halves of labwired_verify can execute.
//
// Usage: node receipt/mcp-attempt.mjs
import { readFileSync, writeFileSync, mkdirSync, copyFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const EXAMPLE = resolve(HERE, '..');
const REPO = process.env.LABWIRED_REPO_ROOT ?? resolve(EXAMPLE, '../../..');
const CLI = process.env.LABWIRED_CLI ?? join(REPO, 'core/target/release/labwired');
const EX = join(HERE, '..');
const FIRMWARE = join(EX, 'firmware/.pio/build/blackpill_f411ce/firmware.elf');

const { Client } = await import(join(REPO, 'node_modules/@modelcontextprotocol/sdk/dist/esm/client/index.js'));
const { StdioClientTransport } = await import(join(REPO, 'node_modules/@modelcontextprotocol/sdk/dist/esm/client/stdio.js'));
const { CallToolResultSchema } = await import(join(REPO, 'node_modules/@modelcontextprotocol/sdk/dist/esm/types.js'));

const elf = readFileSync(FIRMWARE);
const hex = createHash('sha256').update(elf).digest('hex');
const ref = `sha256:${hex}`;

const artifactDir = process.env.LABWIRED_ARTIFACT_DIR ?? join(homedir(), '.labwired', 'artifacts');
mkdirSync(join(artifactDir, 'sha256'), { recursive: true });
copyFileSync(FIRMWARE, join(artifactDir, 'sha256', hex));

const transport = new StdioClientTransport({
  command: join(REPO, 'node_modules/.bin/tsx'),
  args: [join(REPO, 'packages/mcp/src/index.ts')],
  cwd: join(REPO, 'packages/mcp'),
  env: { ...process.env, LABWIRED_CLI: CLI, LABWIRED_REPO_ROOT: REPO },
  stderr: 'pipe',
});

const client = new Client({ name: 'climate-node-receipt-attempt', version: '0.0.0' }, { capabilities: {} });
const out = {
  firmware_ref: ref,
  hosted: {
    attempted: false,
    reason: 'hosted not attempted: no token in this environment',
  },
  labwired_list: null,
  labwired_run: null,
  labwired_verify: null,
};
await client.connect(transport);

// labwired_list through the raw protocol request: the typed callTool refuses a
// content-only result for a tool that declares an outputSchema (client-side
// -32600), so record what the server actually returns.
let listPayload;
try {
  const list = await client.request(
    { method: 'tools/call', params: { name: 'labwired_list', arguments: {} } },
    CallToolResultSchema,
    { timeout: 120000 },
  );
  listPayload = list.structuredContent ?? JSON.parse(list.content[0].text);
} catch (e) {
  listPayload = { client_error: String(e) };
}
const boardIds = Array.isArray(listPayload.boards) ? listPayload.boards.map((b) => b.id) : [];
out.labwired_list = {
  boards: boardIds.length,
  f411_targets: boardIds.filter((id) => /411|climate|f4/i.test(id)),
  note: 'no runnable F411/climate target exists in the stdio board catalog',
};

try {
  const run = await client.callTool(
    { name: 'labwired_run', arguments: { target: 'stm32f411ceu6-climate-node', firmware_ref: ref } },
    undefined,
    { timeout: 300000 },
  );
  out.labwired_run = run.structuredContent ?? JSON.parse(run.content[0].text);
} catch (e) {
  out.labwired_run = { client_error: String(e) };
}

const systemYaml = readFileSync(join(EX, 'system.yaml'), 'utf-8');
const oracle = {
  serial: [
    { contains: 'NODE boot' },
    { contains: 'OLED painted' },
    { matches: 'temp_c=8[0-9]\\.[0-9]{2}' },
  ],
  display: { panel: 'oled-ssd1306', painted: true, min_ink_bytes: 1 },
  gpio: [{ pin: 'PA5', state: 'toggled' }],
};
try {
  const verify = await client.callTool(
    { name: 'labwired_verify', arguments: { system_yaml: systemYaml, firmware_ref: ref, oracle } },
    undefined,
    { timeout: 600000 },
  );
  out.labwired_verify = verify.structuredContent ?? JSON.parse(verify.content[0].text);
} catch (e) {
  out.labwired_verify = { client_error: String(e) };
}

await client.close();
writeFileSync(join(HERE, 'mcp-stdio-attempt.json'), JSON.stringify(out, null, 2) + '\n');
console.log('written receipt/mcp-stdio-attempt.json');
