import { PinSide } from './types';
interface Point {
    x: number;
    y: number;
}
/**
 * Route a wire orthogonally between two pins.
 * Returns a list of waypoints (excluding from/to positions themselves)
 * that form a clean Manhattan path.
 */
export declare function routeWire(from: Point, fromSide: PinSide, to: Point, toSide: PinSide): Point[];
export {};
