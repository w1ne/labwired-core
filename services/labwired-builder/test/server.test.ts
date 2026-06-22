import { describe, it, expect, beforeAll, afterAll, afterEach } from 'vitest';
import { makeServer } from '../src/server';
import { createServer } from 'node:http';
import type { Server } from 'node:http';
import { AddressInfo } from 'node:net';

const SECRET = 'test-secret-1234';

let server: Server;
let port: number;
let base: string;

beforeAll(() => {
  server = makeServer({ secret: SECRET });
  return new Promise<void>((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      port = (server.address() as AddressInfo).port;
      base = `http://127.0.0.1:${port}`;
      resolve();
    });
  });
});

afterAll(() => {
  return new Promise<void>((resolve) => server.close(() => resolve()));
});

describe('server', () => {
  it('/healthz is open without secret', async () => {
    const r = await fetch(`${base}/healthz`);
    expect(r.status).toBe(200);
    const body = await r.json() as { ok: boolean };
    expect(body.ok).toBe(true);
  });

  it('returns 401 without secret header on POST /run', async () => {
    const r = await fetch(`${base}/run`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ elfBase64: 'AA==', systemYaml: '', maxSteps: 100 }),
    });
    expect(r.status).toBe(401);
  });

  it('returns 401 with wrong secret on POST /run', async () => {
    const r = await fetch(`${base}/run`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-builder-secret': 'wrong' },
      body: JSON.stringify({ elfBase64: 'AA==', systemYaml: '', maxSteps: 100 }),
    });
    expect(r.status).toBe(401);
  });

  it('returns 404 for unknown route', async () => {
    const r = await fetch(`${base}/nope`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-builder-secret': SECRET },
      body: JSON.stringify({}),
    });
    expect(r.status).toBe(404);
  });

  it('returns 401 without secret header on POST /run-build', async () => {
    const r = await fetch(`${base}/run-build`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ firmware_base64: 'AA==', system_yaml: 'x', test_yaml: 'y' }),
    });
    expect(r.status).toBe(401);
  });

  it('routes POST /run-build and validates input (400 with ok:false on missing fields, no toolchain needed)', async () => {
    const r = await fetch(`${base}/run-build`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-builder-secret': SECRET },
      body: JSON.stringify({ system_yaml: 'x', test_yaml: 'y' }),
    });
    expect(r.status).toBe(400);
    const body = (await r.json()) as { ok: boolean; error: string };
    expect(body.ok).toBe(false);
    expect(body.error).toMatch(/firmware_base64/i);
  });

  it('routes POST /compile and validates input (200 with ok:false, no toolchain needed)', async () => {
    const r = await fetch(`${base}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-builder-secret': SECRET },
      body: JSON.stringify({ source: '', board: 'stm32l476' }),
    });
    expect(r.status).toBe(200);
    const body = (await r.json()) as { ok: boolean; diagnostics: { message: string }[] };
    expect(body.ok).toBe(false);
    expect(body.diagnostics[0].message).toMatch(/source is required/i);
  });
});

import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls';

describe('GET /chips', () => {
  it('lists the bundled chip ids', async () => {
    const res = await fetch(`${base}/chips`);
    expect(res.status).toBe(200);
    const body = await res.json() as { chips: { id: string }[] };
    const ids = body.chips.map((c: { id: string }) => c.id);
    expect(ids).toEqual(expect.arrayContaining(Object.keys(CHIP_YAMLS)));
  });
});

describe('builder /compile proxy', () => {
  const started: Server[] = [];
  afterEach(async () => {
    delete process.env.COMPILE_URL;
    await Promise.all(started.map((s) => new Promise<void>((r) => s.close(() => r()))));
    started.length = 0;
  });

  async function listen(s: Server): Promise<string> {
    started.push(s);
    await new Promise<void>((r) => s.listen(0, '127.0.0.1', r));
    const addr = s.address();
    if (addr === null || typeof addr === 'string') throw new Error('no port');
    return `http://127.0.0.1:${addr.port}`;
  }

  it('forwards /compile to COMPILE_URL and returns its response', async () => {
    let received: unknown;
    const upstream = createServer((req, res) => {
      const chunks: Buffer[] = [];
      req.on('data', (c) => chunks.push(c));
      req.on('end', () => {
        received = JSON.parse(Buffer.concat(chunks).toString());
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ ok: true, elfBase64: 'ZmFrZQ==', diagnostics: [] }));
      });
    });
    const upstreamBase = await listen(upstream);
    process.env.COMPILE_URL = upstreamBase;

    const builderBase = await listen(makeServer({ secret: 's3cret' }));
    const res = await fetch(`${builderBase}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Builder-Secret': 's3cret' },
      body: JSON.stringify({ board: 'stm32l476', source: 'int main(){}' }),
    });

    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, elfBase64: 'ZmFrZQ==', diagnostics: [] });
    expect(received).toEqual({ board: 'stm32l476', source: 'int main(){}' });
  });

  it('returns 502 when the compile service is unreachable', async () => {
    process.env.COMPILE_URL = 'http://127.0.0.1:1';
    const builderBase = await listen(makeServer({ secret: 's3cret' }));
    const res = await fetch(`${builderBase}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Builder-Secret': 's3cret' },
      body: JSON.stringify({ board: 'stm32l476', source: 'int main(){}' }),
    });
    expect(res.status).toBe(502);
  });
});
