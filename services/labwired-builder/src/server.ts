import { createServer, IncomingMessage, ServerResponse } from 'node:http';
import { run } from './run.js';

const MAX_CONCURRENT = Number(process.env.MAX_CONCURRENT ?? 2);

export interface ServerOptions {
  secret: string;
  port?: number;
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

function json(res: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body);
  res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) });
  res.end(payload);
}

export function makeServer(opts: ServerOptions) {
  let active = 0;

  const server = createServer(async (req: IncomingMessage, res: ServerResponse) => {
    const url = req.url ?? '/';

    // Health check — open, no auth
    if (url === '/healthz') {
      json(res, 200, { ok: true });
      return;
    }

    // All other routes require POST + secret header
    if (req.method !== 'POST') {
      json(res, 405, { error: 'method not allowed' });
      return;
    }

    const providedSecret = req.headers['x-builder-secret'];
    if (providedSecret !== opts.secret) {
      json(res, 401, { error: 'unauthorized' });
      return;
    }

    // Concurrency gate
    if (active >= MAX_CONCURRENT) {
      json(res, 429, { error: 'too many concurrent requests' });
      return;
    }

    active++;
    try {
      const body = await readBody(req);
      let parsed: unknown;
      try {
        parsed = JSON.parse(body);
      } catch {
        json(res, 400, { error: 'invalid JSON body' });
        return;
      }

      if (url === '/run') {
        const req2 = parsed as { elfBase64: string; systemYaml: string; chipYaml?: string; maxSteps: number };
        const result = await run(req2);
        json(res, 200, result);
      } else {
        json(res, 404, { error: 'not found' });
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      json(res, 500, { error: message });
    } finally {
      active--;
    }
  });

  return server;
}

// CLI entry guard
if (process.env.BUILDER_ENTRY === '1') {
  const secret = process.env.BUILDER_SECRET;
  if (!secret) {
    process.stderr.write('BUILDER_SECRET env var is required\n');
    process.exit(1);
  }
  const port = process.env.PORT ? parseInt(process.env.PORT, 10) : 3000;
  const server = makeServer({ secret, port });
  server.listen(port, () => {
    process.stdout.write(`labwired-builder listening on port ${port}\n`);
  });
}
