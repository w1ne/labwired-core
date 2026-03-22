import type { Wire, Part, PinDef, PinSide } from './types';
import { COMPONENT_REGISTRY } from './components/index';
import { routeWire } from './wire-router';

interface WireLayerProps {
  wires: Wire[];
  parts: Part[];
  /** In-progress wire source (rubber-band from this pin to cursor). */
  wireFrom: { part: string; pin: string } | null;
  cursorPos: { x: number; y: number } | null;
  onDeleteWire?: (index: number) => void;
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

export function WireLayer({ wires, parts, wireFrom, cursorPos, onDeleteWire }: WireLayerProps) {
  return (
    <g className="wire-layer">
      {wires.map((wire, i) => {
        const from = resolvePinPos(parts, wire.from.part, wire.from.pin);
        const to = resolvePinPos(parts, wire.to.part, wire.to.pin);
        if (!from || !to) return null;

        // Use stored waypoints or compute orthogonal route
        let waypoints = wire.waypoints;
        if (!waypoints || waypoints.length === 0) {
          const fromSide = resolvePinSide(parts, wire.from.part, wire.from.pin);
          const toSide = resolvePinSide(parts, wire.to.part, wire.to.pin);
          waypoints = routeWire(from, fromSide, to, toSide);
        }

        const allPoints = [from, ...waypoints, to];
        const polyStr = pointsToPolyline(allPoints);

        return (
          <g key={i}>
            <polyline
              points={polyStr}
              fill="none"
              stroke={wire.color}
              strokeWidth={2.5}
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            {/* Invisible wider hitbox for click-to-delete */}
            <polyline
              points={polyStr}
              fill="none"
              stroke="transparent"
              strokeWidth={12}
              style={{ cursor: 'pointer' }}
              onClick={(e) => { e.stopPropagation(); onDeleteWire?.(i); }}
            />
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
