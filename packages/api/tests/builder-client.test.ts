import { describe, it, expect, vi } from 'vitest';
import { builderRun } from '../src/mcp/builder-client.js';

describe('builderRun', () => {
  it('posts to /run with the secret header and forwards systemYaml + chipYaml', async () => {
    const runResponse = {
      status: 'ok', stopReason: 'step_limit', stepsExecuted: 1000, cycles: 1000,
      instructions: 900, serial: '', peripherals: [], timedOut: false,
    };
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(runResponse), { status: 200 }));
    vi.stubGlobal('fetch', fetchMock);
    const env = { BUILDER_URL: 'https://builder.test', BUILDER_SECRET: 'secret' } as any;
    await builderRun(env, { elfBase64: 'AA==', systemYaml: 'sys', chipYaml: 'chip', maxSteps: 1000 });
    const call = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(call[0]).toBe('https://builder.test/run');
    expect((call[1] as any).headers['x-builder-secret']).toBe('secret');
    const body = JSON.parse((call[1] as any).body as string);
    expect(body.systemYaml).toBe('sys');
    expect(body.chipYaml).toBe('chip');
  });
});
