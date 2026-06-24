# Share: reliable short links + mobile Share button

Date: 2026-06-24
Branch: `feat/share-short-mobile` (off `origin/main`)

## Problem

Two issues with sharing labs from the playground:

1. **Full site mints giant links.** Clicking Share on a working example produces a
   multi-kilobyte `#z<base64>` URL instead of a short `?share=<id>` link. Root cause:
   `generateShareUrl` POSTs the lab to `POST /v1/shares`, which runs a **blocking
   validation gate** (`composeDiagnostics`). When the gate fails it returns **422**, and
   the client silently falls back to encoding the entire diagram + source into the URL
   hash. Reproduced live on `app.labwired.com/?board=f103-uds-ecu`: the share POST
   returns `422 DIAGRAM_INVALID` with three **false-positive** errors on a lab that runs:
   - `SCHEMA_PART_UNKNOWN` / `UNKNOWN_COMPONENT` for `logic-analyzer` — the part is a
     real, working instrument but is not registered in `board-config/src/catalog.ts`.
   - `PWR_RAIL_UNDRIVEN` for the `VCC` net — the power rule
     (`board-config/src/erc/power-rules.ts:11`) finds no `power_out` pin because the
     dev-board MCU rail pins (`5V`/`3V3`) do not resolve to `etype: power_out`. The
     `stm32-dev` catalog entry has no pin types at all.

2. **No Share button on mobile.** The desktop Share button lives in
   `studio/TopChrome.tsx` with `hidden sm:flex` (hidden below 640px). On phones the app
   renders `mobile/MobileRunView.tsx`, which has no share affordance at all, so a lab
   cannot be shared from a phone.

## Decisions (from user)

- **Share never blocks on validation.** The human Share button always mints a short
  `?share=<id>` link. Validation becomes a non-blocking signal on that path.
- **Mobile Share = copy link only** (match desktop): copy the short link to clipboard
  and show a toast. No `navigator.share()` native sheet.
- **Also fix the underlying connection/validation false-positives** on the curated
  examples so they are genuinely clean (not merely bypassed).

## Design

### 1. API: decouple sharing from the blocking gate (`packages/api/src/shares.ts`)

`createShareRecord` currently throws `ShareValidationError` on `!validation.ok`, which the
HTTP layer maps to 422. The same function is the storage boundary for **both** the human
share button (`POST /v1/shares`) and the agent-facing MCP `open_hardware_lab` publish path.
The MCP path should keep its false-pass guarantee; the human Share button should not block.

- Add an option to `createShareRecord(env, input, opts?: { enforceValidation?: boolean })`.
  - `enforceValidation: true` (default) → unchanged: throw `ShareValidationError` on
    invalid. Preserves the MCP/agent guarantee — no invalid board is ever *published* by
    an agent.
  - `enforceValidation: false` → still compute `composeDiagnostics`, but **persist anyway**
    and return the diagnostics as a non-blocking `validation` field in the record/response.
- `handleCreateShare` (the share button route) calls with `enforceValidation: false` and
  includes `validation` in the 201 JSON `{ id, url, embed_url, validation }`. It never
  returns 422 for a validation failure (still 400 on malformed JSON / missing diagram).
- The MCP `open_hardware_lab` call site keeps the default (`enforceValidation: true`).

Net effect: the Share button always gets a short link; agent-published labs stay gated.

### 2. Client: keep the hash fallback for *offline only* (`packages/ui/src/editor/sharing.ts`)

With the API no longer 422-ing the share path, `generateShareUrl` returns `body.url`
(the short link) for every well-formed lab. The `#hash` fallback now triggers only on a
true network/API failure (`fetch` throws or non-2xx for reasons other than validation) —
its legitimate offline use. No structural change required; behavior follows from the API
change. (Optional: surface `body.validation` warnings, but the user chose copy-only UX, so
we keep the existing runnable-hint toast in `App.tsx` and do not add new UI.)

### 3. Fix the validator false-positives (`packages/board-config/src/`)

Make the curated examples pass `composeDiagnostics` for real:

- **Register `logic-analyzer` in `catalog.ts`** as an instrument/probe part (mirroring the
  existing `can-diagnostic-tool` / `logic-analyzer` decoder entry in `diagram-contract.ts`),
  with its `CH*`/`GND` pins typed appropriately so `SCHEMA_PART_UNKNOWN` no longer fires
  and its probe pins don't trip power/ERC rules.
- **Make MCU dev-board rail pins drive power.** Ensure the rail pins that examples use to
  power peripherals (`5V`, `3V3`, and the existing `GND`) resolve to `etype: power_out` so
  the `PWR_RAIL_UNDRIVEN` rule recognizes the supply. Fix at the pin-resolution source
  (`catalog.ts` / `pin-mapping.ts`) used by `power-rules.ts`, not by weakening the rule.

### 4. Validate ALL examples (regression harness)

Add a test that builds every bundled example diagram (`makeStarterDiagram` over each
`BoardConfig`, or the existing starter-diagram test infra) and asserts
`composeDiagnostics(diagram).ok === true`. This is the parity gate that prevents the
gate/registry drift from recurring, and it enumerates any remaining connection errors to
fix. Fix every error it surfaces at the root (catalog/pin-map/rules), not per-example.

### 5. Mobile Share button (`packages/playground/src/mobile/MobileRunView.tsx`)

- Add an `onShare?: () => void` prop to `MobileRunViewProps`.
- Render a "Share" button in the existing menu drawer (next to "My projects"), gated on
  `features.menu` — so it appears in the real mobile view but NOT in embeds (embeds use the
  "Open in LabWired" badge instead).
- Wire it in `App.tsx` to the existing `handleShare` callback (same copy-link + toast as
  desktop). No new share logic — reuse `handleShare`.

## Out of scope

- Native `navigator.share()` sheet (user chose copy-only).
- Social/OG preview-image work (separate, already-shipped concern).
- Changing the MCP/agent publish gate behavior.

## Verification

- Unit: new example-validation test green (all bundled examples `ok`).
- API: `POST /v1/shares` with a previously-422 diagram returns 201 + short `url`;
  MCP-path (`enforceValidation: true`) still rejects an invalid diagram.
- Live/preview: share the F103 UDS example → short `?share=` link copied (no `#z...`);
  open it → lab loads. Mobile viewport → Share button visible in menu, copies short link.
- `tsc -b` clean before merge (cross-package type gate is not in the merge CI).
