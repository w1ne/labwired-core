import { createServer, IncomingMessage, ServerResponse } from 'node:http';
import { run } from './run.js';
import { compile, type CompileRequest } from './compile.js';
import { runExample, type RunExampleRequest } from './run-example.js';
import { runBuild, type RunBuildRequest } from './run-build.js';
import { CHIP_YAMLS } from '../../../packages/board-config/src/chip-yamls.js';

const MAX_CONCURRENT = Number(process.env.MAX_CONCURRENT ?? 2);
// Upper bound on a proxied /compile round-trip. The compile service caps its own
// `pio run` at 240s and always returns a result within that, so anything past
// this is a stuck connection — fail fast with 502 instead of hanging the slot.
const COMPILE_PROXY_TIMEOUT_MS = Number(process.env.COMPILE_PROXY_TIMEOUT_MS ?? 250_000);

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

    // Chip catalog — open, no auth
    if (req.method === 'GET' && url === '/chips') {
      const chips = Object.keys(CHIP_YAMLS).sort().map((id) => ({ id }));
      json(res, 200, { chips });
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
      } else if (url === '/compile') {
        // Route to the egress lane only when the request needs to fetch
        // libraries (lib_deps); otherwise the sealed, egress-denied lane.
        const ld = (parsed as { lib_deps?: unknown })?.lib_deps;
        const needsNet = Array.isArray(ld)
          ? ld.length > 0
          : typeof ld === 'string' && ld.trim().length > 0;
        const compileUrl =
          (needsNet && process.env.COMPILE_NET_URL) || process.env.COMPILE_URL;
        if (compileUrl) {
          try {
            const upstream = await fetch(`${compileUrl.replace(/\/$/, '')}/compile`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify(parsed),
              signal: AbortSignal.timeout(COMPILE_PROXY_TIMEOUT_MS),
            });
            const text = await upstream.text();
            res.writeHead(upstream.status, { 'Content-Type': 'application/json' });
            res.end(text);
          } catch (err) {
            const timedOut = err instanceof Error && (err.name === 'TimeoutError' || err.name === 'AbortError');
            const message = err instanceof Error ? err.message : String(err);
            json(res, 502, {
              ok: false,
              error: timedOut
                ? `compile backend timed out after ${COMPILE_PROXY_TIMEOUT_MS}ms`
                : `compile backend unreachable: ${message}`,
            });
          }
        } else {
          const result = await compile(parsed as CompileRequest);
          json(res, 200, result);
        }
      } else if (url === '/run-example') {
        // Run a BAKED-IN example (firmware ELF + manifests are in the image)
        // end-to-end and report the verdict the IO-Link master observed. The
        // example_id is allowlisted + slug-validated in runExample().
        const result = await runExample(parsed as RunExampleRequest);
        json(res, result.ok ? 200 : 400, result);
      } else if (url === '/run-build') {
        // Run a SUPPLIED build (firmware ELF + system manifest + test script,
        // all in the request body — nothing baked in) end-to-end inside the
        // container and report the honest verdict. The generic oracle-run for
        // ANY build; written to an ephemeral, traversal-safe tmp dir.
        const result = await runBuild(parsed as RunBuildRequest);
        json(res, result.ok ? 200 : 400, result);
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
