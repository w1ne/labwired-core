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

  // Escape each pin perpendicular by MARGIN. Because MARGIN (20) > OBSTACLE_MARGIN
  // (8), the exit points lie outside their own component's inflated body, so they
  // are valid A* start/goal nodes even when the endpoint bodies are obstacles.
  const fd = exitDir(fromSide);
  const td = exitDir(toSide);
  const exitFrom = { x: from.x + fd.x * MARGIN, y: from.y + fd.y * MARGIN };
  const exitTo = { x: to.x + td.x * MARGIN, y: to.y + td.y * MARGIN };

  // Route exitFrom -> exitTo on a Hanan grid with A*, avoiding inflated boxes.
  const interior = hananAStar(exitFrom, exitTo, inflated);
  if (interior) {
    return collapseCollinear([exitFrom, ...interior, exitTo]);
  }

  // Fallback: A* found no path (e.g. fully enclosed pin — shouldn't happen for a
  // real board). Return the base path so the wire still renders (best-effort).
  return base;
}

// -- Hanan-grid A* router --

/**
 * Orthogonal A* over a Hanan grid built from the start/goal coordinates and the
 * skirt lines of every inflated obstacle box.
 *
 * Grid construction: candidate X coords = {start.x, goal.x} ∪ {box.x, box.x+w}
 * for every box, plus a 1px channel just outside each vertical box edge so paths
 * can hug the margin without grazing the interior. Same for Y. Nodes are all
 * (x,y) intersections; a node strictly inside any box is blocked.
 *
 * Edges connect grid-adjacent nodes on the same row/column; an edge is allowed
 * only if its segment does not cross any box interior. Cost = Manhattan length +
 * TURN_PENALTY per direction change, so few-bend routes win. Heuristic =
 * Manhattan distance to goal (admissible; turn penalty is extra non-negative
 * cost, so the straight-line Manhattan estimate never overestimates).
 *
 * Returns interior waypoints (EXCLUDING start/goal) or null if unreachable.
 */
const TURN_PENALTY = 1000;
const CHANNEL = 1;

function hananAStar(start: Point, goal: Point, boxes: Box[]): Point[] | null {
  // Build candidate coordinate lines.
  const xs = new Set<number>([start.x, goal.x]);
  const ys = new Set<number>([start.y, goal.y]);
  for (const b of boxes) {
    xs.add(b.x);
    xs.add(b.x + b.w);
    xs.add(b.x - CHANNEL);
    xs.add(b.x + b.w + CHANNEL);
    ys.add(b.y);
    ys.add(b.y + b.h);
    ys.add(b.y - CHANNEL);
    ys.add(b.y + b.h + CHANNEL);
  }
  const xList = [...xs].sort((a, b) => a - b);
  const yList = [...ys].sort((a, b) => a - b);
  const xi = new Map(xList.map((v, i) => [v, i]));
  const yi = new Map(yList.map((v, i) => [v, i]));

  const cols = xList.length;
  const rows = yList.length;
  const nodeId = (ix: number, iy: number) => iy * cols + ix;

  // A grid node is blocked if it lies strictly inside any box interior.
  const blocked = (px: number, py: number): boolean => {
    for (const b of boxes) {
      if (px > b.x + EPS && px < b.x + b.w - EPS && py > b.y + EPS && py < b.y + b.h - EPS) {
        return true;
      }
    }
    return false;
  };

  const startIx = xi.get(start.x)!;
  const startIy = yi.get(start.y)!;
  const goalIx = xi.get(goal.x)!;
  const goalIy = yi.get(goal.y)!;
  const startNode = nodeId(startIx, startIy);
  const goalNode = nodeId(goalIx, goalIy);

  const h = (ix: number, iy: number) =>
    Math.abs(xList[ix] - goal.x) + Math.abs(yList[iy] - goal.y);

  // A* state. dir: 0=none, 1=horizontal, 2=vertical (last move into the node).
  const gScore = new Map<number, number>();
  const cameFrom = new Map<number, number>();
  const cameDir = new Map<number, number>();
  gScore.set(startNode, 0);
  cameDir.set(startNode, 0);

  // Simple binary-heap-free priority queue (arrays are small per wire).
  const open: { id: number; f: number }[] = [{ id: startNode, f: h(startIx, startIy) }];

  const popLowest = (): { id: number; f: number } | undefined => {
    let best = 0;
    for (let i = 1; i < open.length; i++) {
      if (open[i].f < open[best].f) best = i;
    }
    return open.splice(best, 1)[0];
  };

  const neighbors = (ix: number, iy: number): [number, number, number][] => {
    // [ix, iy, dir]
    const out: [number, number, number][] = [];
    if (ix + 1 < cols) out.push([ix + 1, iy, 1]);
    if (ix - 1 >= 0) out.push([ix - 1, iy, 1]);
    if (iy + 1 < rows) out.push([ix, iy + 1, 2]);
    if (iy - 1 >= 0) out.push([ix, iy - 1, 2]);
    return out;
  };

  while (open.length > 0) {
    const cur = popLowest()!;
    if (cur.id === goalNode) break;
    const cix = cur.id % cols;
    const ciy = Math.floor(cur.id / cols);
    const cg = gScore.get(cur.id)!;
    const cdir = cameDir.get(cur.id) ?? 0;

    for (const [nix, niy, ndir] of neighbors(cix, ciy)) {
      const npx = xList[nix];
      const npy = yList[niy];
      if (blocked(npx, npy)) continue;

      // Edge must not cross a box interior.
      const seg: Segment = { a: { x: xList[cix], y: yList[ciy] }, b: { x: npx, y: npy } };
      let crosses = false;
      for (const b of boxes) {
        if (segIntersectsBox(seg, b)) {
          crosses = true;
          break;
        }
      }
      if (crosses) continue;

      const stepLen = Math.abs(npx - xList[cix]) + Math.abs(npy - yList[ciy]);
      const turn = cdir !== 0 && cdir !== ndir ? TURN_PENALTY : 0;
      const tentative = cg + stepLen + turn;

      const nId = nodeId(nix, niy);
      if (tentative < (gScore.get(nId) ?? Infinity)) {
        gScore.set(nId, tentative);
        cameFrom.set(nId, cur.id);
        cameDir.set(nId, ndir);
        open.push({ id: nId, f: tentative + h(nix, niy) });
      }
    }
  }

  if (!gScore.has(goalNode)) return null;

  // Reconstruct path of grid points, then drop the start/goal endpoints.
  const path: Point[] = [];
  let nodeIdCur: number | undefined = goalNode;
  while (nodeIdCur !== undefined) {
    const ix = nodeIdCur % cols;
    const iy = Math.floor(nodeIdCur / cols);
    path.push({ x: xList[ix], y: yList[iy] });
    nodeIdCur = cameFrom.get(nodeIdCur);
  }
  path.reverse();
  // path[0] === start, path[last] === goal; return only interior turn points.
  return path.slice(1, path.length - 1);
}

/** Drop intermediate points that are collinear with their neighbors. */
function collapseCollinear(points: Point[]): Point[] {
  if (points.length <= 2) return points.slice();
  const out: Point[] = [points[0]];
  for (let i = 1; i < points.length - 1; i++) {
    const prev = out[out.length - 1];
    const cur = points[i];
    const next = points[i + 1];
    const collinearH =
      Math.abs(prev.y - cur.y) < EPS && Math.abs(cur.y - next.y) < EPS;
    const collinearV =
      Math.abs(prev.x - cur.x) < EPS && Math.abs(cur.x - next.x) < EPS;
    if (collinearH || collinearV) continue; // skip the redundant midpoint
    out.push(cur);
  }
  out.push(points[points.length - 1]);
  return out;
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
