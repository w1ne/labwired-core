import type { PinSide } from './types';
import { routeAroundObstacles, type Point } from './wire-geometry';

const MARGIN = 20;

/**
 * Compute the initial direction vector for a pin based on its side.
 * The wire exits perpendicular to the component edge.
 * Retained for any callers that import it; wire-geometry owns its own copy.
 */
function exitDir(side: PinSide): Point {
  switch (side) {
    case 'left': return { x: -1, y: 0 };
    case 'right': return { x: 1, y: 0 };
    case 'top': return { x: 0, y: -1 };
    case 'bottom': return { x: 0, y: 1 };
  }
}

/**
 * Route a wire orthogonally between two pins.
 * Returns a list of waypoints (excluding from/to positions themselves)
 * that form a clean Manhattan path.
 *
 * Thin wrapper over `routeAroundObstacles` with no obstacles, preserving the
 * exact path this function produced historically (behavioral superset). Keeps
 * the rubber-band in-progress wire and any other callers working unchanged.
 */
export function routeWire(
  from: Point,
  fromSide: PinSide,
  to: Point,
  toSide: PinSide,
): Point[] {
  return routeAroundObstacles(from, fromSide, to, toSide, []);
}

export { MARGIN, exitDir };
