# Self-Playing Nokia 5110 Hero — Design

**Date:** 2026-06-01
**Status:** Approved (design)
**Area:** `landing_page` (renderer + markup), `core` (offline capture harness)

## Summary

Replace the static, centered landing hero with a **split hero** (text left, Nokia 5110 right) whose LCD loops **real captured framebuffer output** from the actual `nokia5110-invaders-lab` firmware. The loop is recorded once, offline, by running the real firmware on the real `labwired-core`; the landing page ships only a tiny packed frame asset and a vanilla-JS canvas renderer. No WebAssembly or live simulation runs on the page.

This is the "wow" hero: it shows the product's actual output (pixel-for-pixel what the modeled PCD8544 received over SPI) rather than a faked canvas animation, while staying lightweight and crash-proof.

## Goals

- A hero that visibly *moves* on load, anchored to real product output.
- Negligible page weight and no runtime simulation cost.
- Honest labeling: "real captured output," never "live."
- Lean v1; defer interactivity and live sim to a CTA link.

## Non-Goals (v1)

- Live in-hero WebAssembly simulation.
- Click-to-play / interactive control of the game.
- Sound, multiple demos, demo switcher.
- Touching the lower hero content (proof grid, platform pills, trust strip) beyond layout reflow.

## Decision Log

- **Fidelity:** Recorded framebuffer playback (real frames captured at build time), chosen over live wasm (too heavy) and a hand-coded JS game (fake, undercuts the fidelity pitch).
- **Layout:** Split hero — device beside the copy (Option A), chosen over a "see it run" band below (B) and replacing the proof cards (C). The sim is the headline act.
- **Asset format:** A fetched `.bin` asset rather than base64 inlined into HTML — keeps `index.html` clean and compresses well.
- **Loop length:** ~6 s — long enough to read as real play, short enough to stay tiny.

## Architecture

Three units with clean boundaries:

```
invaders.elf → core (offline capture) → invaders-frames.bin → nokia-hero.js → <canvas>
```

### Unit 1 — Capture harness (`core`, offline)

**What it does:** Runs the real invaders firmware in the simulator and emits a packed frame asset. Runs once (manually / on demand), never at page load.

**Where:** A new `#[ignore]`d test or small binary alongside `core/crates/core/tests/e2e_nokia5110_invaders.rs`, reusing its existing helpers (`ensure_firmware_built`, `build_machine`, `framebuffer`).

**How:**
1. Build + load the invaders ELF; construct the `Machine` exactly as the e2e test does.
2. Step the machine. On a fixed cycle cadence (~266k cycles ≈ 15 fps at the 4 MHz MSI clock), call `framebuffer()` (returns 504 bytes) and append to a frame list.
3. **Script the HC-SR04 distance** over the capture window — a slow sweep plus a couple of direction reversals — so the ship visibly tracks back and forth and the loop reads as real play. (The ship X is driven by the echo-pulse loop count, which is deterministic in the sim.)
4. Write `landing_page/assets/invaders-frames.bin`:
   - Header: magic/version, `frame_count` (u16/u32), `width=84`, `height=48`, `fps`.
   - Body: raw concatenation of 504-byte PCD8544 framebuffers (6 banks × 84 columns, each byte = 8 vertical pixels).

**Size budget:** ~6 s × 15 fps ≈ 90 frames × 504 B ≈ **~45 KB raw**; 1-bit data gzips to a few KB over the wire.

**Dependencies:** existing `core` test harness, `labwired-loader`, the built `nokia5110-invaders-lab` ELF.

### Unit 2 — Hero renderer (`landing_page/assets/nokia-hero.js`, vanilla JS)

**What it does:** Fetches the frame asset and plays it on a canvas, matching the plain-HTML/JS stack of the existing landing page.

**How:**
- `fetch()` `invaders-frames.bin` lazily (after first paint / `requestIdleCallback`), parse header, slice frames.
- Decode the banked format (bank `b`, column `x`: byte → 8 pixels at rows `b*8 .. b*8+7`) into an 84×48 buffer; blit to a `<canvas>` scaled up with `image-rendering: pixelated`.
- `requestAnimationFrame` loop advancing frames at the recorded `fps`, looping.
- `IntersectionObserver` pauses the loop when the hero is offscreen.
- `prefers-reduced-motion`: draw a single poster frame, no animation.

**Interface:** Self-initializing on a `<canvas data-nokia-hero>` element; no globals leaked beyond one init call.

**Dependencies:** the `.bin` asset; no third-party libs.

### Unit 3 — Hero markup + CSS (`landing_page/index.html`, `style.css`)

**What it does:** Restructures the top of `.hero` into a two-column grid and adds the device chrome.

**How:**
- Top row becomes a 2-column grid: **left** = existing kicker / H1 / sub / CTAs; **right** = a CSS Nokia 5110 bezel wrapping the `<canvas>` + a caption (`▶ real captured output · invaders.elf`).
- Proof grid, platform pills, and trust strip remain full-width **below** the two-column row, unchanged.
- Collapses to single-column (device beneath text) at mobile breakpoints.
- Add a `Try it live →` CTA pointing to `app.labwired.com` for the real running sim.

**Dependencies:** `nokia-hero.js`, the `.bin` asset.

## Data Flow

Offline (once): `invaders.elf → labwired-core (Machine::step + framebuffer) → invaders-frames.bin` (committed into `landing_page/assets/`).

Runtime (page load): `nokia-hero.js fetch(invaders-frames.bin) → decode banks → canvas (rAF loop)`. No sim, no wasm.

## Error Handling

- Renderer: if the fetch or header parse fails, leave the LCD showing a static poster frame (or a styled blank LCD) — never break the hero layout.
- `prefers-reduced-motion` and offscreen states are first-class, not error paths.

## Testing

- **Capture harness:** assert non-empty output, a sane frame count, and that frames actually differ across the loop (ship moved) — guards against a frozen capture.
- **Decode unit test:** a known bank byte maps to the expected pixel column (8 rows).
- **Manual:** serve `index.html` via `serve.sh`; confirm the loop plays, the reduced-motion fallback shows a static frame, and there's no cumulative layout shift on load.

## Honesty / Copy

- Caption and nearby copy say **"real captured output,"** not "live."
- `Try it live →` links to the actual running sim for visitors who want the real thing.

## Open Items

None blocking. Asset format (fetched `.bin`) and ~6 s loop length are decided defaults, flagged in the design log for easy reversal.
