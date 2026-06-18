# Connection Clarity Overhaul — Design

**Date:** 2026-06-18
**Branch:** `feat/connection-clarity` (off `origin/main`)
**Status:** Approved, ready for implementation

## Problem

In the LabWired diagram editor, a wire that routes near or under a chip *looks*
like it connects to several GPIO pins, because the wire is drawn as a plain
orthogonal polyline that passes geometrically close to pins it has nothing to do
with. Users cannot tell which GPIO a wire actually lands on.

**Key fact:** the data model is already unambiguous. A `Wire` binds two explicit
pin IDs (`from: {part, pin}`, `to: {part, pin}`); `mcuPinForPartPin()` resolves
nets by exact ID match — there is no proximity/geometry inference anywhere. This
is therefore a **pure rendering/interaction problem**. No `Wire` schema change.

## Goal

Make it visually obvious which pin each wire latches to, so a wire crossing a
GPIO row can never be mistaken for connecting to pins it skips.

## Scope (4 features, all in the UI render/interaction layer)

### F1 — Always-on terminal dots (idle baseline)
Every bound endpoint renders a small solid dot at its pin, derived purely from
`wires`. Pins with no wire show no dot. Labels stay hidden at rest. This makes
"the wire latches *here*" visible without interaction and without label clutter.

### F2 — Hover + select endpoint emphasis (core fix)
Symmetric tracing over wires and pins:
- Hover/select a **wire** → its two endpoint pins get a highlight ring +
  enlarged terminal dot; both endpoints show a label (`mcu.PA5`, `led1.1`); the
  wire renders bold/full-opacity; **every other wire and pin dims**.
- Hover/select a **pin** → all wires touching that pin emphasize the same way
  (answers "what is PA5 connected to?").
- Hover previews; click latches (study without holding the mouse). Click empty
  canvas or press `Escape` clears the latched selection. Active resolution:
  `active = hovered ?? selected` (hover wins while hovering; selection persists
  otherwise). Available in `run` mode too (read-only inspection).

### F3 — Hop-overs (disambiguate crossings)
Where a wire segment crosses **another wire's** segment, or passes over a **pin
it does not connect to**, draw a small semicircular hop (the wire "jumps").
A wire crossing a GPIO row visibly hops the pins it skips, so the real endpoint
stands out.
- Deterministic rule: the **horizontal** segment draws the hop over crossing
  **vertical** segments (vertical yields). Stable regardless of wire order.
- Shared-endpoint junctions (segments meeting at a common point) never hop.
- A wire never hops its own segments or its own endpoint pins.

### F4 — Obstacle-aware routing
`routeWire` gains each component's bounding box as an obstacle and routes
*around* chip bodies instead of under them.
- Obstacles = bounding boxes of all parts **except** the wire's own source and
  target parts (those legitimately host the endpoints). Boxes inflated by a
  margin (`OBSTACLE_MARGIN`, ~8px).
- Pragmatic "escape-then-detour", **not** a full A*: exit each pin perpendicular
  by `MARGIN`, attempt the existing L/Z Manhattan path; if any segment crosses
  an inflated obstacle box, take the shorter of the two detours around that box
  (over-the-top/under-the-bottom for a vertical blocker, left/right for a
  horizontal blocker).
- Manual `wire.waypoints` (user-dragged) are still respected as an override and
  bypass auto-routing entirely. Auto-routes are **not** persisted.

## Component boundaries

### `packages/ui/src/editor/wire-geometry.ts` — NEW, pure, unit-tested
No React. Exports:
- `interface Box { x: number; y: number; w: number; h: number }`
- `routeAroundObstacles(from, fromSide, to, toSide, obstacles: Box[]): Point[]`
  — same return contract as today's `routeWire` (waypoints excluding from/to),
  but detours around obstacle boxes. When `obstacles` is empty it must return
  the same path the current `routeWire` produces (behavioral superset).
- `findHops(self: Segment[], others: Segment[], skipPins: Point[]): Hop[]`
  — returns hop markers (point + orientation) for crossings of `others` and for
  `skipPins` the wire passes over but does not terminate on.
- `buildWirePath(points: Point[], hops: Hop[]): string` — SVG `<path>` `d`
  string: straight where there are no hops, small arcs at hop points.
- `segmentsOf(points: Point[]): Segment[]` helper.
- Keep `Point`/`PinSide` exit-direction logic here or import from `wire-router`.

`wire-router.ts` keeps its current `routeWire` (now a thin wrapper:
`routeAroundObstacles(from, fromSide, to, toSide, [])`) so existing callers and
the in-progress rubber-band wire are unaffected.

### `packages/ui/src/editor/WireLayer.tsx`
- Consumes `wire-geometry` to build each wire's path (with hops) and to derive
  terminal dots.
- Renders, per wire: the `<path>` (with hops), a transparent 12px hitbox
  (existing click-to-delete + new hover/click handlers), terminal dots at both
  endpoints, and — when this wire is active — endpoint labels + emphasis styling.
- Dims non-active wires/dots when some other wire or pin is active.
- New props (the contract with EditorCanvas):
  ```ts
  activeWire?: number | null;          // index of emphasized wire
  activePinPartId?: string | null;     // emphasized pin → its wires light up
  activePinId?: string | null;
  onHoverWire?: (index: number | null) => void;
  onSelectWire?: (index: number | null) => void;
  ```
- Terminal-dot/label rendering must reuse `resolvePinPos` (already exported).

### `packages/ui/src/editor/EditorCanvas.tsx`
- Owns interaction state: reuse existing `hoveredPin`/`setHoveredPin`; add
  `hoveredWire`, `selectedWire`, `selectedPin` (`useState`).
- Derive and pass down: `activeWire = hoveredWire ?? selectedWire`;
  `activePin = hoveredPin ?? selectedPin`.
- Wire `WireLayer`'s `onHoverWire`/`onSelectWire` to those setters.
- Pin `<circle>` already has `onMouseEnter/Leave` (hover) — add `onClick` to
  latch `selectedPin` (in addition to the existing wire-draw click path; latch
  only when not currently drawing a wire).
- Clear `selectedWire`/`selectedPin` on empty-canvas click and on `Escape`
  (extend the existing `Escape` handler at ~line 494).
- Emphasis must remain usable in `run` mode for inspection (pins have
  `pointerEvents:none` in run mode, so pin-hover won't fire there — wire hover
  still works; acceptable).

### `packages/ui/src/editor/types.ts`
- Add only UI-only prop/types if shared (e.g. exported `Box`, or co-locate in
  `wire-geometry.ts`). **`Wire` schema unchanged.**

## Data flow
`wires` + `parts` → EditorCanvas resolves `activeWire`/`activePin` from
hover/select state → `WireLayer` → per wire: resolve endpoints → obstacle boxes
(all parts minus the wire's two) → `routeAroundObstacles` (or stored
`waypoints`) → `findHops` vs other wires' segments + unconnected pins →
`buildWirePath` → render path + terminal dots + (if active) labels & emphasis;
(if another wire/pin active) dim.

## Testing
- **Pure unit tests — `wire-geometry.test.ts`** (vitest, fits `src/**/*.test.ts`):
  - `routeAroundObstacles` with empty obstacles === current `routeWire` output
    (golden cases: H-H Z, V-V Z, L-shapes).
  - With one box on the direct path: returned segments do not intersect the
    inflated box; endpoints unchanged.
  - `findHops`: two crossing wires → exactly one hop at the intersection;
    segments sharing an endpoint → zero hops; a wire passing over an unconnected
    pin → one hop; a wire ending on a pin → no hop on that pin.
  - terminal-dot derivation: N wires → correct deduped endpoint set (shared pin
    counted once).
  - `buildWirePath`: no hops → equivalent straight path; with hops → arc command
    present at each hop point.
- **No DOM test env exists** (no jsdom/testing-library). Do NOT add it. The
  React interaction layer (F2 hover/select, dimming, labels) is verified by:
  - `tsc`/build passing,
  - **manual visual inspection in Studio** against a board with a wire crossing
    a GPIO row (the failing case): confirm terminal dots at rest, hop-overs on
    the skipped pins, hover/select emphasis + labels, and dimming. Per repo rule
    "actually use what you ship" — open it and look, don't trust green tests.

## Scope guards (YAGNI)
- No full grid/A* router; no wire-spacing/bus bundling.
- No persisting auto-routed waypoints.
- No `Wire` schema change; no new test infrastructure.
- Per-render cost is O(wires² + wires·pins); fine for board-scale counts.
  Memoization only if profiling later shows need.

## Files touched
- NEW `packages/ui/src/editor/wire-geometry.ts`
- NEW `packages/ui/src/editor/wire-geometry.test.ts`
- `packages/ui/src/editor/wire-router.ts` (thin wrapper delegation)
- `packages/ui/src/editor/WireLayer.tsx`
- `packages/ui/src/editor/EditorCanvas.tsx`
- `packages/ui/src/editor/types.ts` (if shared types needed)
