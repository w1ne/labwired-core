import { createServer, IncomingMessage, ServerResponse } from 'node:http';
import { compile, supportedCompileBoards, supportedChipFamilies, type CompileRequest } from './compile.js';

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

/** Compile service: PlatformIO build only. Reachable only on a private compose
 *  network (never published to host/internet), so auth is optional — enable it
 *  by setting COMPILE_SECRET (checked against the X-Builder-Secret header). */
export function makeCompileServer() {
  const secret = process.env.COMPILE_SECRET;

  return createServer(async (req: IncomingMessage, res: ServerResponse) => {
    const url = req.url ?? '/';

    if (url === '/healthz' || url === '/health') {
      json(res, 200, { ok: true });
      return;
    }
    if (url === '/boards' && req.method === 'GET') {
      json(res, 200, { boards: supportedCompileBoards(), chipFamilies: supportedChipFamilies() });
      return;
    }
    if (url === '/compile' && req.method === 'POST') {
      if (secret && req.headers['x-builder-secret'] !== secret) {
        json(res, 401, { ok: false, error: 'unauthorized' });
        return;
      }
      let parsed: unknown;
      try {
        parsed = JSON.parse(await readBody(req));
      } catch {
        json(res, 400, { ok: false, error: 'invalid JSON body' });
        return;
      }
      const result = await compile(parsed as CompileRequest);
      json(res, 200, result);
      return;
    }
    json(res, 404, { ok: false, error: 'not found' });
  });
}

// Entry guard — only listen when run as the compile image entrypoint.
if (process.env.COMPILE_ENTRY === '1') {
  const port = process.env.PORT ? parseInt(process.env.PORT, 10) : 8080;
  makeCompileServer().listen(port, () => {
    process.stdout.write(`labwired-compile listening on port ${port}\n`);
  });
}
