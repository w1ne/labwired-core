import { Wire, Part } from './types';
interface WireLayerProps {
    wires: Wire[];
    parts: Part[];
    /** In-progress wire source (rubber-band from this pin to cursor). */
    wireFrom: {
        part: string;
        pin: string;
    } | null;
    cursorPos: {
        x: number;
        y: number;
    } | null;
    onDeleteWire?: (index: number) => void;
}
/** Resolve absolute position of a pin on a placed part. */
declare function resolvePinPos(parts: Part[], partId: string, pinId: string): {
    x: number;
    y: number;
} | null;
export declare function WireLayer({ wires, parts, wireFrom, cursorPos, onDeleteWire }: WireLayerProps): import("react/jsx-runtime").JSX.Element;
export { resolvePinPos };
