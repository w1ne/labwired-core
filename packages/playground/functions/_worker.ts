// Advanced Mode worker (bundled to dist/_worker.js at build time).
//
// WHY THIS EXISTS (deploy wiring): CI runs `wrangler pages deploy
// packages/playground/dist` from the repo ROOT. Wrangler discovers the
// file-based `functions/` directory relative to its working directory (the repo
// root), so a `functions/` dir nested at `packages/playground/functions` is NOT
// auto-detected by that command. Cloudflare's documented fallback is Advanced
// Mode: a single `_worker.js` (+ `_routes.json`) emitted into the deploy/output
// directory (`dist/`). `scripts/build-worker.mjs` bundles THIS file to
// `dist/_worker.js`; `public/_routes.json` (copied to `dist/` by Vite) scopes it.
//
// The OG-rewrite logic lives in the pure, unit-tested `shareMeta.ts`. In
// Advanced Mode the origin/static asset is fetched via the `env.ASSETS` binding.
//
// SECURITY: see shareMeta.ts — the `share` id is validated against an exact
// charset/length before use, encodeURIComponent-escaped, and the meta value is
// set via HTMLRewriter setAttribute (escapes). No network/KV/API lookup here.

import { shareImageUrlFor } from './shareMeta';

interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> };
}

const setContent = (value: string) => ({
  element(el: HtmlRewriterElement) {
    el.setAttribute('content', value);
  },
});

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method !== 'GET') return env.ASSETS.fetch(request);

    const url = new URL(request.url);
    const imageUrl = shareImageUrlFor(url.searchParams.get('share'));

    // No (or malformed) share param → serve the origin asset untouched.
    if (!imageUrl) return env.ASSETS.fetch(request);

    const response = await env.ASSETS.fetch(request);
    const contentType = response.headers.get('content-type') || '';
    if (!contentType.includes('text/html')) return response;

    return new HTMLRewriter()
      .on('meta[property="og:image"]', setContent(imageUrl))
      .on('meta[name="twitter:image"]', setContent(imageUrl))
      .on('meta[name="twitter:card"]', setContent('summary_large_image'))
      .transform(response);
  },
};
