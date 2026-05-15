import { describe, expect, it } from 'vitest';
import { spawn } from 'node:child_process';
import { join } from 'node:path';

const DIST = join(import.meta.dirname, '..', 'dist', 'index.js');

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
    setTimeout(() => proc.kill(), 1500);
  });
}

describe('@labwired/mcp stdio server', () => {
  it('responds to initialize + tools/list with 3 tool definitions', async () => {
    const out = await rpcRoundTrip([
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
      { jsonrpc: '2.0', id: 2, method: 'tools/list' },
    ]);
    expect(out).toContain('"name":"labwired_catalog"');
    expect(out).toContain('"name":"labwired_simulate"');
    expect(out).toContain('"name":"labwired_validate_system"');
  });
});
