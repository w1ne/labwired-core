import type { PinSide } from './types';

interface Point {
  x: number;
  y: number;
}

const MARGIN = 20;

/**
 * Compute the initial direction vector for a pin based on its side.
 * The wire exits perpendicular to the component edge.
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
 */
export function routeWire(
  from: Point,
  fromSide: PinSide,
  to: Point,
  toSide: PinSide,
): Point[] {
  const fd = exitDir(fromSide);
  const td = exitDir(toSide);

  // Exit points: extend from each pin by MARGIN in exit direction
  const exitFrom = { x: from.x + fd.x * MARGIN, y: from.y + fd.y * MARGIN };
  const exitTo = { x: to.x + td.x * MARGIN, y: to.y + td.y * MARGIN };

  // Simple case: if both exit horizontally or both vertically,
  // and they can connect with a simple L or Z shape.
  const isHorizFrom = fd.x !== 0;
  const isHorizTo = td.x !== 0;

  if (isHorizFrom && isHorizTo) {
    // Both horizontal exits: connect with a Z-shape (horizontal → vertical → horizontal)
    const midX = (exitFrom.x + exitTo.x) / 2;
    return [
      exitFrom,
      { x: midX, y: exitFrom.y },
      { x: midX, y: exitTo.y },
      exitTo,
    ];
  }

  if (!isHorizFrom && !isHorizTo) {
    // Both vertical exits: connect with a Z-shape (vertical → horizontal → vertical)
    const midY = (exitFrom.y + exitTo.y) / 2;
    return [
      exitFrom,
      { x: exitFrom.x, y: midY },
      { x: exitTo.x, y: midY },
      exitTo,
    ];
  }

  // One horizontal, one vertical: L-shape connection
  if (isHorizFrom && !isHorizTo) {
    // From exits horizontally, To exits vertically → meet at corner
    return [
      exitFrom,
      { x: exitTo.x, y: exitFrom.y },
      exitTo,
    ];
  }

  // From exits vertically, To exits horizontally → meet at corner
  return [
    exitFrom,
    { x: exitFrom.x, y: exitTo.y },
    exitTo,
  ];
}
