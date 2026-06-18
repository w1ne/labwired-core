import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { Server } from 'node:http';
import { makeCompileServer } from '../src/compile-server.js';

let server: Server;
let base: string;

beforeAll(async () => {
  server = makeCompileServer();
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  const addr = server.address();
  if (addr === null || typeof addr === 'string') throw new Error('no port');
  base = `http://127.0.0.1:${addr.port}`;
});

afterAll(() => new Promise<void>((r) => server.close(() => r())));

describe('compile-server', () => {
  it('serves /healthz open', async () => {
    const res = await fetch(`${base}/healthz`);
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true });
  });

  it('aliases /health for proto.cat healthchecks', async () => {
    const res = await fetch(`${base}/health`);
    expect(res.status).toBe(200);
  });

  it('lists boards at /boards', async () => {
    const res = await fetch(`${base}/boards`);
    expect(res.status).toBe(200);
    const body = (await res.json()) as { boards: { board: string; runnable: boolean }[] };
    expect(body.boards.some((b) => b.board === 'stm32l476')).toBe(true);
  });

  it('rejects an unknown board with ok:false (no compiler needed)', async () => {
    const res = await fetch(`${base}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ board: 'not-a-board', source: 'int main(){}' }),
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as { ok: boolean };
    expect(body.ok).toBe(false);
  });

  it('returns 400 on invalid JSON', async () => {
    const res = await fetch(`${base}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{not json',
    });
    expect(res.status).toBe(400);
  });

  it('404s unknown routes', async () => {
    const res = await fetch(`${base}/nope`);
    expect(res.status).toBe(404);
  });
});
