# Per-Lab Share Link Preview Image — Design

**Date:** 2026-06-18
**Branch:** `feat/share-preview-image` (off `main`)
**Status:** Spec for review

## Goal

When a user creates a Share/Embed link, the social link preview (LinkedIn / Slack /
X / iMessage) should show **that lab's own image**, not the generic LabWired card
(#341 shipped the generic `og:image` baseline). Picked approach (brainstorm):
**client renders the canvas to a PNG at share time**, uploads it with the share,
and a Cloudflare **edge function injects a per-share `og:image`**.

## Why an edge function is required

Social crawlers don't run JS; they read the static `<head>`. Today every
`?share=<id>` URL returns the same `index.html`, so the OG tags can't vary per
share without rewriting the HTML at the edge. (Confirmed: the playground is a
static SPA, **no Pages Functions exist yet**.)

## Architecture (3 pieces, current state from investigation)

### A. Client — render canvas → PNG at share time
- `EditorCanvas` already exposes the board `<svg>` via `svgRef`
  (`packages/ui/src/editor/EditorCanvas.tsx:112,510`). Add a helper
  `renderCanvasPng(svg: SVGSVGElement): Promise<string>` (new
  `packages/ui/src/editor/canvasPreview.ts`):
  1. `new XMLSerializer().serializeToString(svg)` → SVG string (inline computed
     sizes; ensure width/height/viewBox present).
  2. Load it into an `Image` (`data:image/svg+xml;base64,…`).
  3. Draw onto an offscreen `<canvas>` sized **1200×630**, letterboxed on the
     brand bg (`#12121a`), lab centered/fit. (Reuses the existing
     `canvas.toDataURL('image/png')` pattern, e.g. `epd-ssd1680-tricolor.tsx:69`.)
  4. Return a PNG data URL.
- `sharing.ts` (`generateShareUrl`/`generateEmbedUrl`): accept an optional
  `previewPng?: string` and include it in the POST body. `App.tsx`
  `handleShare`/`handleEmbed` and `EmbedDialog` render the PNG from `svgRef`
  first and pass it in. **All failures are non-fatal** — if render/upload fails,
  the share still works and falls back to the static `og:image` from #341.

### B. API — store + serve the image (KV, no new infra)
- Shares already live in **KV_PROJECTS** (`shares.ts`, 90-day TTL). KV values may
  be binary up to 25 MB; a 1200×630 PNG is tens of KB. **No R2 binding needed.**
- `POST /v1/shares`: accept optional `preview` (base64 PNG, size-guarded ≤ ~512 KB).
  When present, decode and `KV_PROJECTS.put('shareimg:'+id, bytes, {expirationTtl,
  metadata:{contentType:'image/png'}})`. Reject oversized/non-PNG (magic check).
- New `GET /v1/shares/:id/image` → returns the PNG with `Content-Type: image/png`
  and long `Cache-Control` (immutable; id is content-addressed enough). 404 when
  absent. CORS already `*` for the API.
- Optionally persist the board/lab name on the `ShareRecord` (`title?: string`)
  so the edge can set `og:title` too.

### C. Edge — inject per-share OG meta (new Pages Function)
- Add `packages/playground/functions/_middleware.ts` (Cloudflare Pages
  Functions). For a document request to `/` with a `share` query param, run the
  origin response through **`HTMLRewriter`** and rewrite:
  - `og:image` / `twitter:image` → `https://api.labwired.com/v1/shares/<id>/image`
  - `twitter:card` → `summary_large_image` (now a 1200×630 image)
  - `og:title` / `og:url` → lab name + the share URL (title best-effort: derive
    from the share record via a lightweight API fetch, else leave the default).
  - Non-`share` requests pass through untouched (so the static #341 card remains
    the default everywhere else).
- **Deploy wiring:** `wrangler pages deploy packages/playground/dist` must pick up
  Functions. Confirm the `functions/` dir is discovered from the deploy
  working-dir; if not, fall back to Advanced Mode (`_worker.js` emitted into
  `dist/` + `_routes.json` scoping the function to `/`). This is the one infra
  risk — validate on a Pages preview deployment before relying on it.

## Data flow
Share click → `renderCanvasPng(svgRef)` → POST `/v1/shares {diagram, source,
preview}` → API stores record + `shareimg:<id>` → returns `url`. Crawler later
fetches `…/?share=<id>` → Pages Function HTMLRewrites `og:image` →
`…/v1/shares/<id>/image` → card shows the lab.

## Testing
- **Unit (ui):** `canvasPreview` — given a stub `<svg>`, returns a PNG data URL of
  the target dimensions. (jsdom lacks a real canvas; gate the DOM-dependent path
  and unit-test the SVG-serialize + sizing math, mocking `canvas`/`Image`.)
- **Unit (api):** `POST /v1/shares` stores the image and rejects oversized/non-PNG;
  `GET /v1/shares/:id/image` returns bytes + `image/png` + 404 when missing.
- **Edge:** HTMLRewriter rewrite is integration-tested on a Pages preview
  (`wrangler pages dev`); assert `og:image` points at the image endpoint for a
  `?share=` URL and is untouched otherwise.
- **Manual:** validate the real card with LinkedIn Post Inspector / opengraph.xyz
  after deploy (per "actually use what you ship").

## Deploy surfaces (note: TWO pipelines)
- API change → `api-worker-deploy.yml` (separate worker deploy).
- Playground + Pages Function → `pages-deploy.yml`.
- Ship order: **API first** (image store/serve must exist before the edge
  references it), then the Pages Function + client. Otherwise cards 404 briefly.

## Cost / efficiency (hard constraint)
- **No standing infra cost:** KV blob (no R2 bucket to provision/bill).
- **Near-zero origin reads on serve:** `GET /v1/shares/:id/image` sends
  `Cache-Control: public, max-age=31536000, immutable` → Cloudflare CDN caches it,
  so repeated crawler/unfurl hits don't reach the worker/KV.
- **Edge function does NO lookups:** it builds `og:image` purely from the `<id>`
  in the URL (`…/v1/shares/<id>/image`) — no KV/API fetch per crawl. `og:title`
  rewrite is DROPPED (it would cost a fetch); the title stays the #341 default.
- **Zero cost on normal loads:** `_routes.json` scopes the function to the HTML
  document only (excludes `/assets/*`, wasm, icons); and it early-returns
  (pass-through, no HTMLRewriter) when there's no `share` param.
- **Image gen is free:** rendered in the user's browser at share time.

## Scope guards (YAGNI)
- No R2 (KV blob is enough); no headless screenshot service; no live-state capture
  (static authored view); no per-embed theming; no `og:title` rewrite (costs a
  fetch). One write on share-create, cached reads thereafter.

## Files
- NEW `packages/ui/src/editor/canvasPreview.ts` (+ test)
- `packages/ui/src/editor/sharing.ts` (optional `previewPng` in POST)
- `packages/playground/src/App.tsx`, `EmbedDialog.tsx` (render + pass preview)
- `packages/api/src/shares.ts`, `src/index.ts` (store image, GET image route) (+ tests)
- NEW `packages/playground/functions/_middleware.ts` (edge OG rewrite) (+ `_routes.json`/`_worker.js` if needed)
