import { COMPONENT_REGISTRY } from './components/index';
import type { Diagram } from './types';

export interface DiagramBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Axis-aligned bounding box over every part in a diagram, in SVG/world
 * coordinates — the space EditorCanvas's viewBox lives in.
 *
 * Mirrors the part transform in EditorCanvas: a part renders at (x, y) with an
 * inner `scale(sc) rotate(rotate, w/2, h/2)`, so the footprint is the registry
 * width/height rotated about its own centre, scaled about the part origin, then
 * translated. A 90°/270° rotation swaps the effective width and height.
 *
 * Returns null when the diagram has no registry-known parts to bound.
 */
export function computeDiagramBounds(diagram: Diagram): DiagramBounds | null {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;

  for (const part of diagram.parts) {
    const def = COMPONENT_REGISTRY.get(part.type);
    if (!def) continue;

    const sc = part.scale ?? 1;
    const w0 = def.width;
    const h0 = def.height;
    // Rotation pivots about the local centre, so the centre is invariant.
    const cx = w0 / 2;
    const cy = h0 / 2;
    const rot = (((part.rotate ?? 0) % 360) + 360) % 360;
    const swap = rot === 90 || rot === 270;
    const exX = (swap ? h0 : w0) / 2;
    const exY = (swap ? w0 : h0) / 2;

    minX = Math.min(minX, part.x + sc * (cx - exX));
    minY = Math.min(minY, part.y + sc * (cy - exY));
    maxX = Math.max(maxX, part.x + sc * (cx + exX));
    maxY = Math.max(maxY, part.y + sc * (cy + exY));
  }

  if (!Number.isFinite(minX)) return null;
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}
