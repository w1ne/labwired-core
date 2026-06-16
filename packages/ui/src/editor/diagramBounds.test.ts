import { describe, it, expect } from 'vitest';
import { computeDiagramBounds } from './diagramBounds';
import { COMPONENT_REGISTRY } from './components/index';
import type { Diagram, Part } from './types';

const led = COMPONENT_REGISTRY.get('led')!;

function diagram(parts: Part[]): Diagram {
  return { version: 1, board: 'test', parts, wires: [] };
}

function part(over: Partial<Part>): Part {
  return { id: 'p', type: 'led', x: 0, y: 0, rotate: 0, attrs: {}, ...over };
}

describe('computeDiagramBounds', () => {
  it('returns null for an empty diagram', () => {
    expect(computeDiagramBounds(diagram([]))).toBeNull();
  });

  it('ignores parts whose type is not in the registry', () => {
    expect(computeDiagramBounds(diagram([part({ type: 'made-up' })]))).toBeNull();
  });

  it('bounds a single unrotated part to its registry footprint at its origin', () => {
    const b = computeDiagramBounds(diagram([part({ x: 10, y: 20 })]))!;
    expect(b).toEqual({ x: 10, y: 20, width: led.width, height: led.height });
  });

  it('scales the footprint about the part origin', () => {
    const b = computeDiagramBounds(diagram([part({ x: 0, y: 0, scale: 2 })]))!;
    expect(b).toEqual({ x: 0, y: 0, width: led.width * 2, height: led.height * 2 });
  });

  it('swaps width and height for a 90° rotation, keeping the centre fixed', () => {
    const b = computeDiagramBounds(diagram([part({ x: 0, y: 0, rotate: 90 })]))!;
    // Rotation pivots about the centre (w/2, h/2), so the box recentres there.
    const cx = led.width / 2;
    const cy = led.height / 2;
    expect(b.width).toBeCloseTo(led.height);
    expect(b.height).toBeCloseTo(led.width);
    expect(b.x + b.width / 2).toBeCloseTo(cx);
    expect(b.y + b.height / 2).toBeCloseTo(cy);
  });

  it('unions multiple parts', () => {
    const b = computeDiagramBounds(
      diagram([part({ id: 'a', x: 0, y: 0 }), part({ id: 'b', x: 100, y: 50 })]),
    )!;
    expect(b.x).toBe(0);
    expect(b.y).toBe(0);
    expect(b.width).toBe(100 + led.width);
    expect(b.height).toBe(50 + led.height);
  });
});
