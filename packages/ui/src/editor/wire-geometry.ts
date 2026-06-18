import type { PinSide } from './types';

// -- Geometry primitives --

export interface Point {
  x: number;
  y: number;
}

/** Axis-aligned bounding box, top-left origin. */
export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type Orientation = 'h' | 'v';

/** Axis-aligned segment (a and b share either x or y). */
export interface Segment {
  a: Point;
  b: Point;
}

/** Hop marker sitting on a segment of the given orientation. */
export interface Hop {
  at: Point;
  on: Orientation;
}

/** Perpendicular exit distance from a pin before routing turns. */
export const MARGIN = 20;

/** Inflation applied to obstacle boxes on every side. */
export const OBSTACLE_MARGIN = 8;

/** Tolerance for "passes over" a skip pin. */
const PIN_TOLERANCE = 6;

/** Hop arc radius. */
const HOP_RADIUS = 5;

const EPS = 1e-6;

// -- Exit direction --

/**
 * Initial direction vector for a pin based on its side. The wire exits
 * perpendicular to the component edge. Kept in sync with wire-router.ts.
 */
function exitDir(side: PinSide): Point {
  switch (side) {
    case 'left': return { x: -1, y: 0 };
    case 'right': return { x: 1, y: 0 };
    case 'top': return { x: 0, y: -1 };
    case 'bottom': return { x: 0, y: 1 };
  }
}

// -- segmentsOf --

/** Convert a polyline of points into N-1 axis-aligned segments. */
export function segmentsOf(points: Point[]): Segment[] {
  const segs: Segment[] = [];
  for (let i = 0; i + 1 < points.length; i++) {
    segs.push({ a: points[i], b: points[i + 1] });
  }
  return segs;
}

// -- Base routing (behavioral copy of routeWire) --

/**
 * Base Manhattan path identical to wire-router.routeWire. Returns waypoints
 * excluding from/to.
 */
function baseRoute(
  from: Point,
  fromSide: PinSide,
  to: Point,
  toSide: PinSide,
): Point[] {
  const fd = exitDir(fromSide);
  const td = exitDir(toSide);

  const exitFrom = { x: from.x + fd.x * MARGIN, y: from.y + fd.y * MARGIN };
  const exitTo = { x: to.x + td.x * MARGIN, y: to.y + td.y * MARGIN };

  const isHorizFrom = fd.x !== 0;
  const isHorizTo = td.x !== 0;

  if (isHorizFrom && isHorizTo) {
    const midX = (exitFrom.x + exitTo.x) / 2;
    return [
      exitFrom,
      { x: midX, y: exitFrom.y },
      { x: midX, y: exitTo.y },
      exitTo,
    ];
  }

  if (!isHorizFrom && !isHorizTo) {
    const midY = (exitFrom.y + exitTo.y) / 2;
    return [
      exitFrom,
      { x: exitFrom.x, y: midY },
      { x: exitTo.x, y: midY },
      exitTo,
    ];
  }

  if (isHorizFrom && !isHorizTo) {
    return [
      exitFrom,
      { x: exitTo.x, y: exitFrom.y },
      exitTo,
    ];
  }

  return [
    exitFrom,
    { x: exitFrom.x, y: exitTo.y },
    exitTo,
  ];
}

// -- Obstacle helpers --

function inflate(box: Box, m: number): Box {
  return { x: box.x - m, y: box.y - m, w: box.w + 2 * m, h: box.h + 2 * m };
}

/** True if an axis-aligned segment overlaps a box (treating the segment as its
 *  bounding rectangle — zero-thickness lines still count if they cross). */
function segIntersectsBox(s: Segment, box: Box): boolean {
  const minX = Math.min(s.a.x, s.b.x);
  const maxX = Math.max(s.a.x, s.b.x);
  const minY = Math.min(s.a.y, s.b.y);
  const maxY = Math.max(s.a.y, s.b.y);
  const bx0 = box.x;
  const bx1 = box.x + box.w;
  const by0 = box.y;
  const by1 = box.y + box.h;
  return maxX > bx0 + EPS && minX < bx1 - EPS && maxY > by0 + EPS && minY < by1 - EPS;
}

function anySegmentBlocked(points: Point[], boxes: Box[]): boolean {
  const segs = segmentsOf(points);
  for (const s of segs) {
    for (const b of boxes) {
      if (segIntersectsBox(s, b)) return true;
    }
  }
  return false;
}

/** First box (already inflated) that any segment of the path crosses. */
function firstBlocker(points: Point[], boxes: Box[]): Box | null {
  const segs = segmentsOf(points);
  for (const s of segs) {
    for (const b of boxes) {
      if (segIntersectsBox(s, b)) return b;
    }
  }
  return null;
}

// -- routeAroundObstacles --

/**
 * Route a wire orthogonally between two pins, detouring around obstacle boxes.
 * Behavioral superset of routeWire: with `obstacles=[]` returns exactly the
 * same waypoints. Returns waypoints EXCLUDING from/to.
 */
export function routeAroundObstacles(
  from: Point,
  fromSide: PinSide,
  to: Point,
  toSide: PinSide,
  obstacles: Box[],
): Point[] {
  const base = baseRoute(from, fromSide, to, toSide);

  if (!obstacles || obstacles.length === 0) {
    return base;
  }

  const inflated = obstacles.map((b) => inflate(b, OBSTACLE_MARGIN));

  // If the base path (with endpoints) is clear, keep it unchanged.
  const baseFull = [from, ...base, to];
  if (!anySegmentBlocked(baseFull, inflated)) {
    return base;
  }

  // Escape pins perpendicular by MARGIN, then attempt an L/Z detour around the
  // blocking box. We work between the two exit points and prepend/append them.
  const fd = exitDir(fromSide);
  const td = exitDir(toSide);
  const exitFrom = { x: from.x + fd.x * MARGIN, y: from.y + fd.y * MARGIN };
  const exitTo = { x: to.x + td.x * MARGIN, y: to.y + td.y * MARGIN };

  // Candidate interior paths between exitFrom and exitTo, ordered by preference.
  // Each candidate is the list of interior waypoints (between exits).
  const candidates = buildDetourCandidates(exitFrom, exitTo, inflated);

  let best: Point[] | null = null;
  let bestLen = Infinity;
  for (const interior of candidates) {
    const full = [from, exitFrom, ...interior, exitTo, to];
    if (anySegmentBlocked(full, inflated)) continue;
    const len = pathLength(full);
    if (len < bestLen) {
      bestLen = len;
      best = [exitFrom, ...interior, exitTo];
    }
  }

  if (best) return best;

  // Fallback: couldn't find a fully-clear detour; return the base path so the
  // wire still renders (best-effort for hard multi-blocker cases).
  return base;
}

/**
 * Build a set of candidate interior routes (between the two exit points) that
 * attempt to skirt the blocking box(es). Pragmatic escape-then-detour, not A*.
 */
function buildDetourCandidates(
  exitFrom: Point,
  exitTo: Point,
  inflated: Box[],
): Point[][] {
  const candidates: Point[][] = [];

  // The straight L/Z connectors between the two exits (no detour).
  // Z via mid-X (horizontal-dominant)
  const midX = (exitFrom.x + exitTo.x) / 2;
  candidates.push([
    { x: midX, y: exitFrom.y },
    { x: midX, y: exitTo.y },
  ]);
  // Z via mid-Y (vertical-dominant)
  const midY = (exitFrom.y + exitTo.y) / 2;
  candidates.push([
    { x: exitFrom.x, y: midY },
    { x: exitTo.x, y: midY },
  ]);
  // L corners
  candidates.push([{ x: exitTo.x, y: exitFrom.y }]);
  candidates.push([{ x: exitFrom.x, y: exitTo.y }]);

  // Detours around the blocking box. Identify the box crossing the simplest
  // direct connector and route over/under or left/right of it.
  const direct = [exitFrom, { x: exitTo.x, y: exitFrom.y }, exitTo];
  const blocker = firstBlocker(direct, inflated) ?? inflated[0];
  if (blocker) {
    const bx0 = blocker.x;
    const bx1 = blocker.x + blocker.w;
    const by0 = blocker.y;
    const by1 = blocker.y + blocker.h;

    // Over-the-top / under-the-bottom (route in Y around a vertical-blocking box):
    // go to a Y just outside the box (top or bottom), traverse X, then come back.
    const overY = by0 - OBSTACLE_MARGIN; // above the box
    const underY = by1 + OBSTACLE_MARGIN; // below the box
    candidates.push([
      { x: exitFrom.x, y: overY },
      { x: exitTo.x, y: overY },
    ]);
    candidates.push([
      { x: exitFrom.x, y: underY },
      { x: exitTo.x, y: underY },
    ]);

    // Left / right (route in X around a horizontal-blocking box).
    const leftX = bx0 - OBSTACLE_MARGIN;
    const rightX = bx1 + OBSTACLE_MARGIN;
    candidates.push([
      { x: leftX, y: exitFrom.y },
      { x: leftX, y: exitTo.y },
    ]);
    candidates.push([
      { x: rightX, y: exitFrom.y },
      { x: rightX, y: exitTo.y },
    ]);
  }

  return candidates;
}

function pathLength(points: Point[]): number {
  let len = 0;
  for (let i = 0; i + 1 < points.length; i++) {
    len += Math.abs(points[i + 1].x - points[i].x) + Math.abs(points[i + 1].y - points[i].y);
  }
  return len;
}

// -- findHops --

function isHorizontal(s: Segment): boolean {
  return Math.abs(s.a.y - s.b.y) < EPS && Math.abs(s.a.x - s.b.x) > EPS;
}

function isVertical(s: Segment): boolean {
  return Math.abs(s.a.x - s.b.x) < EPS && Math.abs(s.a.y - s.b.y) > EPS;
}

function samePoint(p: Point, q: Point): boolean {
  return Math.abs(p.x - q.x) < EPS && Math.abs(p.y - q.y) < EPS;
}

function sharesEndpoint(h: Segment, v: Segment): boolean {
  return (
    samePoint(h.a, v.a) ||
    samePoint(h.a, v.b) ||
    samePoint(h.b, v.a) ||
    samePoint(h.b, v.b)
  );
}

/**
 * Crossing point of a horizontal and a vertical segment, only if they cross at
 * an INTERIOR point of both (strictly between endpoints). Returns null if they
 * merely touch at an endpoint or do not cross.
 */
function interiorCross(h: Segment, v: Segment): Point | null {
  const y = h.a.y; // horizontal segment's constant y
  const x = v.a.x; // vertical segment's constant x
  const hMinX = Math.min(h.a.x, h.b.x);
  const hMaxX = Math.max(h.a.x, h.b.x);
  const vMinY = Math.min(v.a.y, v.b.y);
  const vMaxY = Math.max(v.a.y, v.b.y);

  // x must be strictly inside the horizontal span, y strictly inside vertical.
  if (x > hMinX + EPS && x < hMaxX - EPS && y > vMinY + EPS && y < vMaxY - EPS) {
    return { x, y };
  }
  return null;
}

function pointOnHorizSegmentInterior(p: Point, s: Segment): boolean {
  if (!isHorizontal(s)) return false;
  if (Math.abs(p.y - s.a.y) > PIN_TOLERANCE) return false;
  const minX = Math.min(s.a.x, s.b.x);
  const maxX = Math.max(s.a.x, s.b.x);
  return p.x > minX + EPS && p.x < maxX - EPS;
}

function pointOnVertSegmentInterior(p: Point, s: Segment): boolean {
  if (!isVertical(s)) return false;
  if (Math.abs(p.x - s.a.x) > PIN_TOLERANCE) return false;
  const minY = Math.min(s.a.y, s.b.y);
  const maxY = Math.max(s.a.y, s.b.y);
  return p.y > minY + EPS && p.y < maxY - EPS;
}

function pointIsSegmentEndpoint(p: Point, s: Segment): boolean {
  return (
    (Math.abs(p.x - s.a.x) <= PIN_TOLERANCE && Math.abs(p.y - s.a.y) <= PIN_TOLERANCE) ||
    (Math.abs(p.x - s.b.x) <= PIN_TOLERANCE && Math.abs(p.y - s.b.y) <= PIN_TOLERANCE)
  );
}

/**
 * Find hop markers. A hop is produced where a self-segment truly crosses an
 * other-segment at an interior point of both (one horizontal, one vertical),
 * and where a self-segment passes over a skip pin it does not terminate on.
 * Per spec the hop sits on the HORIZONTAL segment (vertical yields).
 */
export function findHops(
  self: Segment[],
  others: Segment[],
  skipPins: Point[],
): Hop[] {
  const hops: Hop[] = [];

  for (const s of self) {
    for (const o of others) {
      // Need exactly one horizontal and one vertical, crossing.
      let h: Segment | null = null;
      let v: Segment | null = null;
      if (isHorizontal(s) && isVertical(o)) {
        h = s;
        v = o;
      } else if (isVertical(s) && isHorizontal(o)) {
        h = o;
        v = s;
      }
      if (!h || !v) continue;
      if (sharesEndpoint(h, v)) continue;
      const cross = interiorCross(h, v);
      if (cross) {
        hops.push({ at: cross, on: 'h' });
      }
    }
  }

  // Skip pins: a self-segment passes over a pin it does not terminate on.
  for (const pin of skipPins) {
    // Never hop a pin the wire actually terminates on (endpoint of a segment).
    if (self.some((s) => pointIsSegmentEndpoint(pin, s))) continue;

    for (const s of self) {
      if (pointOnHorizSegmentInterior(pin, s)) {
        hops.push({ at: { x: pin.x, y: s.a.y }, on: 'h' });
        break;
      }
      if (pointOnVertSegmentInterior(pin, s)) {
        hops.push({ at: { x: s.a.x, y: pin.y }, on: 'v' });
        break;
      }
    }
  }

  return hops;
}

// -- buildWirePath --

function fmt(n: number): string {
  // Trim to avoid floating noise while keeping precision.
  return Number.isInteger(n) ? String(n) : String(Math.round(n * 1000) / 1000);
}

/**
 * Build an SVG path `d` string. Straight polyline where there are no hops; a
 * small semicircular arc (r≈HOP_RADIUS) at each hop point. Hops are applied to
 * the segment whose orientation matches the hop.
 */
export function buildWirePath(points: Point[], hops: Hop[]): string {
  if (points.length === 0) return '';
  if (points.length === 1) {
    return `M ${fmt(points[0].x)} ${fmt(points[0].y)}`;
  }

  const segs = segmentsOf(points);
  let d = `M ${fmt(points[0].x)} ${fmt(points[0].y)}`;

  for (const seg of segs) {
    const horizontal = isHorizontal(seg);
    const vertical = isVertical(seg);

    // Hops that belong on this segment, ordered along the direction of travel.
    const segHops = hops.filter((hp) => hopOnSegment(hp, seg, horizontal, vertical));

    if (horizontal) {
      const dir = Math.sign(seg.b.x - seg.a.x) || 1;
      segHops.sort((p, q) => dir * (p.at.x - q.at.x));
    } else if (vertical) {
      const dir = Math.sign(seg.b.y - seg.a.y) || 1;
      segHops.sort((p, q) => dir * (p.at.y - q.at.y));
    }

    for (const hp of segHops) {
      d += emitHop(hp, seg, horizontal);
    }

    // Final straight to segment end.
    d += ` L ${fmt(seg.b.x)} ${fmt(seg.b.y)}`;
  }

  return d;
}

function hopOnSegment(hp: Hop, seg: Segment, horizontal: boolean, vertical: boolean): boolean {
  if (hp.on === 'h' && !horizontal) return false;
  if (hp.on === 'v' && !vertical) return false;
  if (horizontal) {
    const minX = Math.min(seg.a.x, seg.b.x);
    const maxX = Math.max(seg.a.x, seg.b.x);
    return (
      Math.abs(hp.at.y - seg.a.y) < PIN_TOLERANCE &&
      hp.at.x > minX + EPS &&
      hp.at.x < maxX - EPS
    );
  }
  if (vertical) {
    const minY = Math.min(seg.a.y, seg.b.y);
    const maxY = Math.max(seg.a.y, seg.b.y);
    return (
      Math.abs(hp.at.x - seg.a.x) < PIN_TOLERANCE &&
      hp.at.y > minY + EPS &&
      hp.at.y < maxY - EPS
    );
  }
  return false;
}

/**
 * Emit a line up to just before the hop, then a semicircular arc over it.
 * `sweep` chooses the bulge side; horizontal hops bulge upward, vertical hops
 * bulge to one side, but the precise side is cosmetic.
 */
function emitHop(hp: Hop, seg: Segment, horizontal: boolean): string {
  const r = HOP_RADIUS;
  if (horizontal) {
    const dir = Math.sign(seg.b.x - seg.a.x) || 1;
    const startX = hp.at.x - dir * r;
    const endX = hp.at.x + dir * r;
    const y = hp.at.y;
    // sweep-flag 1 bulges "up" (negative y) for left-to-right travel.
    const sweep = dir > 0 ? 1 : 0;
    return ` L ${fmt(startX)} ${fmt(y)} A ${r} ${r} 0 0 ${sweep} ${fmt(endX)} ${fmt(y)}`;
  }
  // vertical
  const dir = Math.sign(seg.b.y - seg.a.y) || 1;
  const startY = hp.at.y - dir * r;
  const endY = hp.at.y + dir * r;
  const x = hp.at.x;
  const sweep = dir > 0 ? 0 : 1;
  return ` L ${fmt(x)} ${fmt(startY)} A ${r} ${r} 0 0 ${sweep} ${fmt(x)} ${fmt(endY)}`;
}
