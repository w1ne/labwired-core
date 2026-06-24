import { describe, it, expect } from 'vitest';
import {
  routeAroundObstacles,
  findHops,
  buildWirePath,
  segmentsOf,
  OBSTACLE_MARGIN,
  type Point,
  type Box,
  type Segment,
  type Hop,
} from './wire-geometry';
import { routeWire } from './wire-router';

describe('segmentsOf', () => {
  it('N points => N-1 segments with correct coords', () => {
    const pts: Point[] = [
      { x: 0, y: 0 },
      { x: 10, y: 0 },
      { x: 10, y: 5 },
    ];
    const segs = segmentsOf(pts);
    expect(segs).toHaveLength(2);
    expect(segs[0]).toEqual({ a: { x: 0, y: 0 }, b: { x: 10, y: 0 } });
    expect(segs[1]).toEqual({ a: { x: 10, y: 0 }, b: { x: 10, y: 5 } });
  });

  it('returns [] for fewer than 2 points', () => {
    expect(segmentsOf([])).toEqual([]);
    expect(segmentsOf([{ x: 1, y: 2 }])).toEqual([]);
  });
});

describe('routeAroundObstacles with no obstacles === routeWire (golden)', () => {
  it('H-H (right -> left)', () => {
    const from = { x: 0, y: 0 };
    const to = { x: 200, y: 50 };
    expect(routeAroundObstacles(from, 'right', to, 'left', [])).toEqual(
      routeWire(from, 'right', to, 'left'),
    );
  });

  it('H-H (left -> right)', () => {
    const from = { x: 200, y: 0 };
    const to = { x: 0, y: 80 };
    expect(routeAroundObstacles(from, 'left', to, 'right', [])).toEqual(
      routeWire(from, 'left', to, 'right'),
    );
  });

  it('V-V (top -> bottom)', () => {
    const from = { x: 10, y: 0 };
    const to = { x: 60, y: 200 };
    expect(routeAroundObstacles(from, 'top', to, 'bottom', [])).toEqual(
      routeWire(from, 'top', to, 'bottom'),
    );
  });

  it('V-V (bottom -> top)', () => {
    const from = { x: 10, y: 200 };
    const to = { x: 60, y: 0 };
    expect(routeAroundObstacles(from, 'bottom', to, 'top', [])).toEqual(
      routeWire(from, 'bottom', to, 'top'),
    );
  });

  it('L-shape (horizontal from, vertical to)', () => {
    const from = { x: 0, y: 0 };
    const to = { x: 100, y: 100 };
    expect(routeAroundObstacles(from, 'right', to, 'top', [])).toEqual(
      routeWire(from, 'right', to, 'top'),
    );
  });

  it('L-shape (vertical from, horizontal to)', () => {
    const from = { x: 0, y: 0 };
    const to = { x: 100, y: 100 };
    expect(routeAroundObstacles(from, 'top', to, 'left', [])).toEqual(
      routeWire(from, 'top', to, 'left'),
    );
  });
});

// helper: does an axis-aligned segment intersect a box (any overlap)?
function segIntersectsBox(s: Segment, box: Box): boolean {
  const minX = Math.min(s.a.x, s.b.x);
  const maxX = Math.max(s.a.x, s.b.x);
  const minY = Math.min(s.a.y, s.b.y);
  const maxY = Math.max(s.a.y, s.b.y);
  const bx0 = box.x;
  const bx1 = box.x + box.w;
  const by0 = box.y;
  const by1 = box.y + box.h;
  // overlap test (treating the segment as its bounding rect)
  return maxX > bx0 && minX < bx1 && maxY > by0 && minY < by1;
}

function inflateBox(box: Box, m: number): Box {
  return { x: box.x - m, y: box.y - m, w: box.w + 2 * m, h: box.h + 2 * m };
}

/** Are two consecutive points axis-aligned (share x or y)? */
function isOrthogonal(a: Point, b: Point): boolean {
  return Math.abs(a.x - b.x) < 1e-6 || Math.abs(a.y - b.y) < 1e-6;
}

describe('routeAroundObstacles with a blocking box', () => {
  it('no returned segment intersects the inflated box; endpoints unchanged', () => {
    // Two horizontally-facing pins with a chip body straddling the direct path.
    const from = { x: 0, y: 50 };
    const to = { x: 300, y: 50 };
    // Box centered on the straight line between them.
    const box: Box = { x: 120, y: 20, w: 60, h: 60 };
    const inflated = inflateBox(box, OBSTACLE_MARGIN);

    const wps = routeAroundObstacles(from, 'right', to, 'left', [box]);

    // Full path including endpoints (endpoints excluded from return per contract)
    const full: Point[] = [from, ...wps, to];
    for (const s of segmentsOf(full)) {
      expect(segIntersectsBox(s, inflated)).toBe(false);
    }

    // Endpoints never moved: first waypoint is not the endpoint, but the path
    // must still begin at `from` and end at `to` (we prepend/append them).
    expect(full[0]).toEqual(from);
    expect(full[full.length - 1]).toEqual(to);
  });

  it('blinky-shaped case: routes AROUND the MCU body, never under it', () => {
    // A large MCU. The FROM pin sits on its LEFT edge; the TO pin (an LED) is
    // to the RIGHT of the whole MCU body. A naive direct route would cut
    // straight through the chip.
    const mcu: Box = { x: 100, y: 100, w: 200, h: 160 };
    const from = { x: 100, y: 180 }; // left edge of MCU
    const to = { x: 400, y: 180 }; // LED pin to the right of MCU, left-facing

    const wps = routeAroundObstacles(from, 'left', to, 'left', [mcu]);
    const full: Point[] = [from, ...wps, to];

    // Goes around, not under: no segment crosses the MCU body interior. We test
    // against the RAW body (not the inflated box): the pin's own perpendicular
    // exit stub necessarily passes through the inflation margin band of its own
    // component, but the wire must never cut across the actual chip body.
    for (const s of segmentsOf(full)) {
      expect(segIntersectsBox(s, mcu)).toBe(false);
    }
    // Connected end-to-end.
    expect(full[0]).toEqual(from);
    expect(full[full.length - 1]).toEqual(to);
  });

  it('returned path is orthogonal and connects from -> to', () => {
    const from = { x: 0, y: 50 };
    const to = { x: 300, y: 50 };
    const box: Box = { x: 120, y: 20, w: 60, h: 60 };

    const wps = routeAroundObstacles(from, 'right', to, 'left', [box]);
    const full: Point[] = [from, ...wps, to];

    expect(full[0]).toEqual(from);
    expect(full[full.length - 1]).toEqual(to);
    for (const s of segmentsOf(full)) {
      expect(isOrthogonal(s.a, s.b)).toBe(true);
    }
  });

  it('turn-penalty sanity: a clear straight shot returns minimal bends', () => {
    // Facing pins with a clear shot and an obstacle well out of the way; the
    // router must not introduce gratuitous bends. baseRoute (H-H) yields a
    // 4-point Z; with a turn penalty an unobstructed shot stays simple.
    const from = { x: 0, y: 50 };
    const to = { x: 300, y: 50 };
    const farBox: Box = { x: 100, y: 400, w: 60, h: 60 }; // far below, no effect
    const wps = routeAroundObstacles(from, 'right', to, 'left', [farBox]);
    const full: Point[] = [from, ...wps, to];
    // Count direction changes.
    let bends = 0;
    const segs = segmentsOf(full);
    for (let i = 1; i < segs.length; i++) {
      const prevH = Math.abs(segs[i - 1].a.y - segs[i - 1].b.y) < 1e-6;
      const curH = Math.abs(segs[i].a.y - segs[i].b.y) < 1e-6;
      if (prevH !== curH) bends++;
    }
    // from and to share the same y (50), so the ideal path is one straight
    // line: zero bends.
    expect(bends).toBe(0);
  });
});

describe('findHops', () => {
  it('two crossing wires => exactly one hop on the horizontal segment at the crossing', () => {
    // self has a horizontal segment; other has a vertical segment crossing it.
    const self: Segment[] = [{ a: { x: 0, y: 50 }, b: { x: 100, y: 50 } }];
    const others: Segment[] = [{ a: { x: 50, y: 0 }, b: { x: 50, y: 100 } }];
    const hops = findHops(self, others, []);
    expect(hops).toHaveLength(1);
    expect(hops[0].on).toBe('h');
    expect(hops[0].at).toEqual({ x: 50, y: 50 });
  });

  it('segments sharing an endpoint => no hop', () => {
    const self: Segment[] = [{ a: { x: 0, y: 50 }, b: { x: 50, y: 50 } }];
    const others: Segment[] = [{ a: { x: 50, y: 50 }, b: { x: 50, y: 100 } }];
    expect(findHops(self, others, [])).toEqual([]);
  });

  it('self segment passing over a skip pin => one hop at the pin', () => {
    const self: Segment[] = [{ a: { x: 0, y: 50 }, b: { x: 100, y: 50 } }];
    const pin: Point = { x: 40, y: 50 };
    const hops = findHops(self, [], [pin]);
    expect(hops).toHaveLength(1);
    expect(hops[0].at).toEqual({ x: 40, y: 50 });
  });

  it('a pin that is an endpoint of self => no hop', () => {
    const self: Segment[] = [{ a: { x: 0, y: 50 }, b: { x: 100, y: 50 } }];
    const endpointPin: Point = { x: 0, y: 50 };
    expect(findHops(self, [], [endpointPin])).toEqual([]);
  });

  it('no crossing => no hops', () => {
    const self: Segment[] = [{ a: { x: 0, y: 50 }, b: { x: 100, y: 50 } }];
    const others: Segment[] = [{ a: { x: 200, y: 0 }, b: { x: 200, y: 100 } }];
    expect(findHops(self, others, [])).toEqual([]);
  });
});

describe('buildWirePath', () => {
  const pts: Point[] = [
    { x: 0, y: 0 },
    { x: 100, y: 0 },
    { x: 100, y: 50 },
  ];

  it('no hops => no arc command and visits every point', () => {
    const d = buildWirePath(pts, []);
    expect(d).not.toContain('A');
    expect(d).toContain('M 0 0');
    expect(d).toContain('100 0');
    expect(d).toContain('100 50');
  });

  it('one hop => exactly one arc command present', () => {
    const hops: Hop[] = [{ at: { x: 50, y: 0 }, on: 'h' }];
    const d = buildWirePath(pts, hops);
    const arcs = (d.match(/A/g) || []).length;
    expect(arcs).toBe(1);
  });
});
