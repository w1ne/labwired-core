# Embed Button — Branded iframe embed of a lab

**Date:** 2026-06-18
**Branch:** `feat/embed-button` (off `main`, which already has the run-only embed foundation)
**Status:** Approved (brainstorm), ready for implementation

## Problem / goal

Let anyone drop a **live, interactive (run-only) LabWired lab** into a web page —
our own docs/landing first, any site in general. Today the Share button only
copies a share link; there's no way to get embed code, and the embedded view
carries no branding. Add an **Embed** action that produces branded, copy-paste
`<iframe>` code, with a LabWired badge inside the embed that links back.

## Decisions (locked in brainstorm)

- **Form:** branded **iframe** snippet — NOT a web component. (For a heavy WASM
  sim, even a "proper" embed iframes under the hood — isolation, no style/security
  bleed; the YouTube/CodePen/Wokwi pattern.)
- **Share button stays simple** — its one-click copy-link behavior is untouched.
- **Separate Embed button** next to Share opens the embed flow.
- **Run-only**: embeds load `?embed=true`, which the foundation PR already maps to
  `interactionMode='run'` (read-only; buttons/sliders only).
- **Branding:** a small **corner badge** (logo + "Made with LabWired") pinned in
  the embed, linking back to the full lab.

## Components (isolation)

1. **Embed button** — `TopChrome.tsx` gains an optional `onEmbed?: () => void`
   prop, rendered as a button immediately left of Share, using a **secondary**
   style (ghost/subtle, not the accent fill Share uses) so Share stays the
   primary action. Wired through the same prop chain as `onShare`.

2. **`handleEmbed` + dialog state** — `App.tsx` adds `embedOpen` state and a
   `handleEmbed` that opens the dialog; passes `onEmbed={() => setEmbedOpen(true)}`
   down to `TopChrome`. Mirrors the existing `handleShare`/`onShare` wiring.

3. **`EmbedDialog.tsx`** (new, `packages/playground/src/studio/`) — on open:
   - calls existing `generateEmbedUrl(diagram, source)` → `…/?embed=true&share=<id>`
     (hash fallback offline, same path as Share); shows a loading state while
     generating and a graceful error/toast on failure.
   - renders a **copy-paste snippet** built by `buildEmbedSnippet` (below),
   - a **Copy** button (`navigator.clipboard`, with copied confirmation),
   - a **live preview**: a real `<iframe src={embedUrl}>` so the user sees exactly
     what embeds,
   - **height presets**: Compact (420px) / Tall (600px); width is always 100%.

4. **`buildEmbedSnippet(url, opts)`** (new pure helper, e.g.
   `packages/playground/src/studio/embedSnippet.ts`) — returns the responsive
   wrapper + `<iframe>` markup string:
   ```html
   <iframe src="<url>" title="LabWired lab" width="100%" height="<h>"
     style="border:0;border-radius:8px" loading="lazy"
     sandbox="allow-scripts allow-same-origin allow-popups"></iframe>
   ```
   Pure, deterministic, unit-tested.

5. **`EmbedBadge.tsx`** (new, `packages/playground/src/`) — rendered in the app
   **only when `isEmbedMode()`**: `GlobalLogo` + "Made with LabWired", pinned
   bottom-right, small and semi-transparent. Links to the full lab = the current
   URL with the `embed` param removed (deep-links to the editable view). Reuses
   the existing `GlobalLogo` component / `public/logo.svg`. `target="_blank"
   rel="noopener"`.

## Data flow
Click **Embed** → `handleEmbed` opens `EmbedDialog` → `generateEmbedUrl` creates a
share (or hash) → `buildEmbedSnippet(url, {height})` → user copies. Independently,
any page loaded with `?embed=true` renders `EmbedBadge` over the run-only canvas,
linking back to the full lab.

## Out of scope for v1 (flagged; not built)
- **OG meta tags + preview thumbnail** for social cards (the LinkedIn/Twitter
  story — LinkedIn can't iframe; it shows an OG card). Worth a small follow-up
  slice (`og:image` from an auto screenshot) but separate from the Embed button.
- Web component / `<labwired-lab>`, `postMessage` two-way API, per-embed theming,
  arbitrary custom dimensions (presets only).

## Testing
- **Unit (vitest):** `buildEmbedSnippet` — given url + each height preset → exact
  expected `<iframe>` markup; sandbox + loading attrs present; url is attribute-
  escaped.
- **Component (RTL — playground already has jsdom `.test.tsx`):**
  - `EmbedBadge` renders the logo, text, and an anchor whose href is the current
    URL minus `embed`, `target=_blank rel=noopener`.
  - `EmbedDialog` shows the snippet text and the Copy button invokes clipboard
    (mock `navigator.clipboard.writeText`).
- `tsc` clean + playground prod build.
- **Visual check in the preview:** Embed button → dialog → snippet + live preview;
  confirm the corner badge shows and links back in `?embed=true`. (Per repo rule
  "actually use what you ship" — open it, don't trust green tests.)

## Files
- NEW `packages/playground/src/studio/EmbedDialog.tsx`
- NEW `packages/playground/src/studio/embedSnippet.ts` + `embedSnippet.test.ts`
- NEW `packages/playground/src/EmbedBadge.tsx` (+ `EmbedBadge.test.tsx`)
- `packages/playground/src/studio/TopChrome.tsx` (Embed button + `onEmbed` prop)
- `packages/playground/src/App.tsx` (`embedOpen` state, `handleEmbed`, render
  `EmbedDialog` + `EmbedBadge`, pass `onEmbed`)
