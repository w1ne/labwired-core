// Bundle the Advanced Mode Pages worker (functions/_worker.ts) into
// dist/_worker.js so `wrangler pages deploy packages/playground/dist` ships the
// per-share og:image rewrite.
//
// WHY Advanced Mode (not the file-based functions/ convention): CI runs
// `wrangler pages deploy packages/playground/dist` from the repo ROOT. Wrangler
// resolves the file-based `functions/` dir relative to its working directory
// (repo root), so `packages/playground/functions/` is NOT auto-detected by that
// command. Cloudflare's documented fallback is a single `_worker.js` in the
// output dir + a `_routes.json` (Vite copies public/_routes.json into dist/).
//
// Run after `vite build` (see the package.json "build" script).

import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import fs from 'node:fs';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..');
const entry = path.join(root, 'functions', '_worker.ts');
const outfile = path.join(root, 'dist', '_worker.js');

if (!fs.existsSync(path.join(root, 'dist'))) {
  console.error('[build-worker] dist/ not found — run `vite build` first.');
  process.exit(1);
}

await build({
  entryPoints: [entry],
  outfile,
  bundle: true,
  format: 'esm',
  target: 'es2022',
  platform: 'neutral',
  // HTMLRewriter, Request, Response, URL are Workers runtime globals — leave them.
});

console.log(`[build-worker] wrote ${path.relative(root, outfile)}`);
