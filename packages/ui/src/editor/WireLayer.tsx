import type { Wire, Part, PinDef, PinSide } from './types';
import { COMPONENT_REGISTRY } from './components/index';
import { routeWire } from './wire-router';
import {
  routeAroundObstacles,
  findHops,
  buildWirePath,
  segmentsOf,
  type Box,
  type Point,
} from './wire-geometry';

interface WireLayerProps {
  wires: Wire[];
  parts: Part[];
  /** In-progress wire source (rubber-band from this pin to cursor). */
  wireFrom: { part: string; pin: string } | null;
  cursorPos: { x: number; y: number } | null;
  onDeleteWire?: (index: number) => void;
  /** Index of the emphasized wire (hover wins over selection upstream). */
  activeWire?: number | null;
  /** Emphasized pin: every wire touching it lights up. */
  activePinPartId?: string | null;
  activePinId?: string | null;
  onHoverWire?: (index: number | null) => void;
  onSelectWire?: (index: number | null) => void;
}

/** Resolve absolute position of a pin on a placed part. */
function resolvePinPos(
  parts: Part[],
  partId: string,
  pinId: string,
): { x: number; y: number } | null {
  const part = parts.find((p) => p.id === partId);
  if (!part) return null;
  const def = COMPONENT_REGISTRY.get(part.type);
  if (!def) return null;
  const pin = def.pins.find((p: PinDef) => p.id === pinId);
  if (!pin) return null;

  // Apply scale then rotation around component center
  const sc = part.scale ?? 1;
  const cx = def.width / 2;
  const cy = def.height / 2;
  const px = (pin.x - cx) * sc;
  const py = (pin.y - cy) * sc;
  const rad = ((part.rotate || 0) * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const rx = px * cos - py * sin;
  const ry = px * sin + py * cos;

  return { x: part.x + cx * sc + rx, y: part.y + cy * sc + ry };
}

/** Resolve the effective side of a pin after component rotation. */
function resolvePinSide(parts: Part[], partId: string, pinId: string): PinSide {
  const part = parts.find((p) => p.id === partId);
  if (!part) return 'right';
  const def = COMPONENT_REGISTRY.get(part.type);
  if (!def) return 'right';
  const pin = def.pins.find((p: PinDef) => p.id === pinId);
  if (!pin) return 'right';

  const sides: PinSide[] = ['top', 'right', 'bottom', 'left'];
  const baseIdx = sides.indexOf(pin.side);
  const rotSteps = Math.round(((part.rotate || 0) % 360) / 90);
  return sides[(baseIdx + rotSteps) % 4];
}

/** Build an SVG polyline points string from a list of points. */
function pointsToPolyline(pts: { x: number; y: number }[]): string {
  return pts.map((p) => `${p.x},${p.y}`).join(' ');
}

/** Pin label (`part.pin`) for an endpoint, falling back to the pin id. */
function pinLabelFor(parts: Part[], partId: string, pinId: string): string {
  const part = parts.find((p) => p.id === partId);
  const def = part ? COMPONENT_REGISTRY.get(part.type) : undefined;
  const pin = def?.pins.find((p: PinDef) => p.id === pinId);
  return `${partId}.${pin?.label ?? pinId}`;
}

/** Outward text offset for an endpoint label, based on the resolved pin side. */
function labelOffset(side: PinSide): { dx: number; dy: number; anchor: 'start' | 'middle' | 'end' } {
  switch (side) {
    case 'left': return { dx: -8, dy: 3, anchor: 'end' };
    case 'right': return { dx: 8, dy: 3, anchor: 'start' };
    case 'top': return { dx: 0, dy: -8, anchor: 'middle' };
    case 'bottom': return { dx: 0, dy: 14, anchor: 'middle' };
  }
}

/** Bounding box (raw, uninflated) of a placed part in canvas space. */
function partBox(part: Part): Box | null {
  const def = COMPONENT_REGISTRY.get(part.type);
  if (!def) return null;
  const sc = part.scale ?? 1;
  let w = def.width * sc;
  let h = def.height * sc;
  // Axis-aligned best-effort: swap dimensions for quarter-turn rotations.
  const rotSteps = Math.round(((part.rotate || 0) % 360) / 90) % 4;
  if (rotSteps === 1 || rotSteps === 3) {
    [w, h] = [h, w];
  }
  return { x: part.x, y: part.y, w, h };
}

export function WireLayer({
  wires,
  parts,
  wireFrom,
  cursorPos,
  onDeleteWire,
  activeWire = null,
  activePinPartId = null,
  activePinId = null,
  onHoverWire,
  onSelectWire,
}: WireLayerProps) {
  // Pre-resolve full point lists per wire (used both for hop computation across
  // wires and for rendering). Wires whose endpoints can't be resolved are null.
  const resolved = wires.map((wire) => {
    const from = resolvePinPos(parts, wire.from.part, wire.from.pin);
    const to = resolvePinPos(parts, wire.to.part, wire.to.pin);
    if (!from || !to) return null;

    const fromSide = resolvePinSide(parts, wire.from.part, wire.from.pin);
    const toSide = resolvePinSide(parts, wire.to.part, wire.to.pin);

    let waypoints = wire.waypoints;
    if (!waypoints || waypoints.length === 0) {
      // Obstacles = EVERY part body, including this wire's own source/target
      // components. A pin sits on its component's edge, so excluding the
      // endpoint bodies used to let the wire route UNDER the very chip it
      // connects to. The router escapes each pin to an exit point MARGIN (20)
      // outside the edge — beyond the OBSTACLE_MARGIN (8) inflation — so the
      // exit is clear of its own inflated body and the wire visibly routes
      // around it instead of through it.
      const boxes: Box[] = [];
      for (const part of parts) {
        const b = partBox(part);
        if (b) boxes.push(b);
      }
      waypoints = routeAroundObstacles(from, fromSide, to, toSide, boxes);
    }

    const points: Point[] = [from, ...waypoints, to];
    return { from, to, fromSide, toSide, points };
  });

  // Absolute positions of every pin on every part (for skip-pin hop detection).
  const allPins: { partId: string; pinId: string; pos: Point }[] = [];
  for (const part of parts) {
    const def = COMPONENT_REGISTRY.get(part.type);
    if (!def) continue;
    for (const pin of def.pins) {
      const pos = resolvePinPos(parts, part.id, pin.id);
      if (pos) allPins.push({ partId: part.id, pinId: pin.id, pos });
    }
  }

  const somethingActive =
    activeWire != null || (activePinPartId != null && activePinId != null);

  return (
    <g className="wire-layer">
      {wires.map((wire, i) => {
        const r = resolved[i];
        if (!r) return null;
        const { from, to, fromSide, toSide, points } = r;

        // Is this wire emphasized? Either explicitly active, or it touches the
        // active pin at one of its endpoints.
        const touchesActivePin =
          activePinPartId != null &&
          activePinId != null &&
          ((wire.from.part === activePinPartId && wire.from.pin === activePinId) ||
            (wire.to.part === activePinPartId && wire.to.pin === activePinId));
        const isActive = i === activeWire || touchesActivePin;
        const dimmed = somethingActive && !isActive;

        // skipPins: all pins EXCEPT this wire's two endpoints.
        const skipPins: Point[] = allPins
          .filter(
            (p) =>
              !(p.partId === wire.from.part && p.pinId === wire.from.pin) &&
              !(p.partId === wire.to.part && p.pinId === wire.to.pin),
          )
          .map((p) => p.pos);

        // others: segments of every OTHER resolved wire.
        const selfSegs = segmentsOf(points);
        const others = resolved
          .filter((o, oi) => o != null && oi !== i)
          .flatMap((o) => segmentsOf(o!.points));

        const hops = findHops(selfSegs, others, skipPins);
        const d = buildWirePath(points, hops);

        const opacity = dimmed ? 0.25 : 1;
        const strokeWidth = isActive ? 3.5 : 2.5;
        const dotR = isActive ? 4.5 : 3;

        const fromOff = labelOffset(fromSide);
        const toOff = labelOffset(toSide);

        return (
          <g key={i}>
            <path
              d={d}
              fill="none"
              stroke={wire.color}
              strokeWidth={strokeWidth}
              strokeLinecap="round"
              strokeLinejoin="round"
              opacity={opacity}
              pointerEvents="none"
            />
            {/* Invisible wider hitbox: single-click selects, shift-click deletes. */}
            <path
              d={d}
              fill="none"
              stroke="transparent"
              strokeWidth={12}
              style={{ cursor: 'pointer' }}
              onClick={(e) => {
                e.stopPropagation();
                if (e.shiftKey) {
                  onDeleteWire?.(i);
                } else {
                  onSelectWire?.(i);
                }
              }}
              onMouseEnter={() => onHoverWire?.(i)}
              onMouseLeave={() => onHoverWire?.(null)}
            />
            {/* F1 terminal dots */}
            <circle
              cx={from.x}
              cy={from.y}
              r={dotR}
              fill={wire.color}
              opacity={opacity}
              pointerEvents="none"
            />
            <circle
              cx={to.x}
              cy={to.y}
              r={dotR}
              fill={wire.color}
              opacity={opacity}
              pointerEvents="none"
            />
            {/* F2 active emphasis: highlight rings + endpoint labels */}
            {isActive && (
              <>
                <circle
                  cx={from.x}
                  cy={from.y}
                  r={dotR + 2.5}
                  fill="none"
                  stroke="#fff"
                  strokeWidth={1.5}
                  pointerEvents="none"
                />
                <circle
                  cx={to.x}
                  cy={to.y}
                  r={dotR + 2.5}
                  fill="none"
                  stroke="#fff"
                  strokeWidth={1.5}
                  pointerEvents="none"
                />
                <text
                  x={from.x + fromOff.dx}
                  y={from.y + fromOff.dy}
                  fill="#fff"
                  fontFamily="'JetBrains Mono', monospace"
                  fontSize={10}
                  fontWeight={700}
                  textAnchor={fromOff.anchor}
                  stroke="#1a1a2e"
                  strokeWidth={3}
                  paintOrder="stroke"
                  pointerEvents="none"
                >
                  {pinLabelFor(parts, wire.from.part, wire.from.pin)}
                </text>
                <text
                  x={to.x + toOff.dx}
                  y={to.y + toOff.dy}
                  fill="#fff"
                  fontFamily="'JetBrains Mono', monospace"
                  fontSize={10}
                  fontWeight={700}
                  textAnchor={toOff.anchor}
                  stroke="#1a1a2e"
                  strokeWidth={3}
                  paintOrder="stroke"
                  pointerEvents="none"
                >
                  {pinLabelFor(parts, wire.to.part, wire.to.pin)}
                </text>
              </>
            )}
          </g>
        );
      })}
      {/* Rubber-band wire in progress */}
      {wireFrom && cursorPos && (() => {
        const from = resolvePinPos(parts, wireFrom.part, wireFrom.pin);
        if (!from) return null;
        const fromSide = resolvePinSide(parts, wireFrom.part, wireFrom.pin);
        // Compute live orthogonal route to cursor
        const liveWaypoints = routeWire(from, fromSide, cursorPos, 'left');
        const allPoints = [from, ...liveWaypoints, cursorPos];
        return (
          <polyline
            points={pointsToPolyline(allPoints)}
            fill="none"
            stroke="#e83e8c"
            strokeWidth={2}
            strokeDasharray="6,4"
            strokeLinejoin="round"
            pointerEvents="none"
          />
        );
      })()}
    </g>
  );
}

export { resolvePinPos };
