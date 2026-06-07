import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { makeServer } from '../src/server';
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
    const r = await fetch(`${base}/compile`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'x-builder-secret': SECRET },
      body: JSON.stringify({}),
    });
    expect(r.status).toBe(404);
  });
});
